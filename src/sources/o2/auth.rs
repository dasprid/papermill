use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use inquire::{Password, PasswordDisplayMode};
use reqwest::StatusCode;
use reqwest::cookie::{CookieStore, Jar};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use url::Url;

use crate::credentials::UsernamePasswordCredentials;
use crate::keystore;
use crate::sources::{SourceError, SourceKind};

const DEVICE_JWT_ACCOUNT: &str = "source:o2:device-jwt";

fn read_device_jwt() -> anyhow::Result<Option<SecretString>> {
    keystore::read_secret(DEVICE_JWT_ACCOUNT)
}

fn write_device_jwt(value: &SecretString) -> anyhow::Result<()> {
    keystore::write_secret(DEVICE_JWT_ACCOUNT, value)
}

use super::callbacks::{
    AuthChainResponse, Callback, InProgressAuth, confirmation_option_index, default_choice,
    find_by_id, find_by_type, find_pow_script,
};

const LOGIN_HOST: &str = "login.o2online.de";
const AUTHENTICATE_URL: &str = "https://login.o2online.de/signin/json/realms/root/realms/o2/authenticate?goto=https%3A%2F%2Fwww.o2online.de%2Fmein-o2%2F";
const PORTAL_AUTHENTICATE_URL: &str = "https://www.o2online.de/vt-login/authenticate/";
const OAUTH_CLIENT_ID: &str = "U-469-b2c_portal_care";
const CIAM_COOKIE_NAME: &str = "ciamsessionid";
const DEVICE_JWT_COOKIE_NAME: &str = "device-jwt-c";

const KIND: SourceKind = SourceKind::O2;
use super::pow;

const STAGE_LOGIN: &str = "stage-loginLegacy";
const STAGE_PASSWORD: &str = "stage-passwordInput";
const STAGE_MSISDN_CHOICE: &str = "stage-selectMSISDN1";
const SMS_STAGE_PREFIX: &str = "stage-smsLoginPage";

fn fill_common(callbacks: &mut [Callback]) -> anyhow::Result<()> {
    if let Some(script) = find_pow_script(callbacks) {
        let nonce = pow::solve(&script)?;

        if let Some(callback) = find_by_id(callbacks, "proofOfWorkNonce") {
            callback.set_input_value(Value::String(nonce));
        }
    }

    if let Some(callback) = find_by_id(callbacks, "_remember_od") {
        callback.set_input_value(Value::String("checked".to_string()));
    }

    Ok(())
}

fn fill_confirmation_by_name(
    callbacks: &mut [Callback],
    option: &str,
    stage: &str,
) -> anyhow::Result<()> {
    let confirm = find_by_type(callbacks, "ConfirmationCallback")
        .with_context(|| format!("o2: ConfirmationCallback missing in {stage}"))?;
    let index = confirmation_option_index(confirm, option).with_context(|| {
        format!(r#"o2: option "{option}" not found in {stage} ConfirmationCallback"#)
    })?;
    confirm.set_input_value(Value::Number(index.into()));
    Ok(())
}

fn fill_login(
    callbacks: &mut [Callback],
    credentials: &UsernamePasswordCredentials,
) -> anyhow::Result<()> {
    fill_common(callbacks)?;

    let name_callback = find_by_type(callbacks, "NameCallback")
        .context("Failed to find NameCallback in login stage")?;
    name_callback.set_input_value(Value::String(credentials.username.clone()));

    fill_confirmation_by_name(callbacks, "custom.o2.common.continue", STAGE_LOGIN)?;

    Ok(())
}

fn fill_password(
    callbacks: &mut [Callback],
    credentials: &UsernamePasswordCredentials,
) -> anyhow::Result<()> {
    fill_common(callbacks)?;

    let password_callback = find_by_type(callbacks, "PasswordCallback")
        .context("Failed to find PasswordCallback in password stage")?;
    password_callback.set_input_value(Value::String(
        credentials.password.expose_secret().to_string(),
    ));

    fill_confirmation_by_name(
        callbacks,
        "custom.o2.loginuserbasic.loginbtn",
        STAGE_PASSWORD,
    )?;

    Ok(())
}

fn fill_msisdn_choice(callbacks: &mut [Callback]) -> anyhow::Result<()> {
    if let Some(choice) = find_by_type(callbacks, "ChoiceCallback") {
        let default = default_choice(choice);
        choice.set_input_value(Value::Number(default.into()));
    }

    fill_confirmation_by_name(
        callbacks,
        "custom.o2.common.sendSMSNow",
        STAGE_MSISDN_CHOICE,
    )?;

    Ok(())
}

fn fill_sms(callbacks: &mut [Callback]) -> anyhow::Result<()> {
    fill_common(callbacks)?;

    let name_callback = find_by_type(callbacks, "NameCallback")
        .context("Failed to find NameCallback in SMS stage")?;

    if !io::stdin().is_terminal() {
        bail!("o2: SMS code required but stdin is not a TTY");
    }

    let code = Password::new(&format!("[{}] SMS code:", KIND.name()))
        .with_display_mode(PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .context("Failed to read SMS code")?;

    name_callback.set_input_value(Value::String(code));

    fill_confirmation_by_name(callbacks, "custom.o2.common.verify", "stage-smsLoginPage")?;

    Ok(())
}

fn fill_stage(
    response: &mut InProgressAuth,
    credentials: &UsernamePasswordCredentials,
) -> anyhow::Result<()> {
    let stage = response.stage.as_deref().unwrap_or("");

    if stage == STAGE_LOGIN {
        return fill_login(&mut response.callbacks, credentials);
    }

    if stage == STAGE_PASSWORD {
        return fill_password(&mut response.callbacks, credentials);
    }

    if stage == STAGE_MSISDN_CHOICE {
        return fill_msisdn_choice(&mut response.callbacks);
    }

    if stage.starts_with(SMS_STAGE_PREFIX) {
        return fill_sms(&mut response.callbacks);
    }

    bail!("o2: unhandled auth stage \"{stage}\"")
}

fn auth_headers(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    builder
        .header("accept", "application/json, text/javascript, */*; q=0.01")
        .header("accept-language", "en-US")
        .header("accept-api-version", "protocol=1.0,resource=2.1")
        .header("x-requested-with", "XMLHttpRequest")
        .header("x-username", "anonymous")
        .header("x-password", "anonymous")
        .header("x-nosession", "true")
        .header("content-type", "application/json")
}

async fn post_authenticate(
    client: &reqwest::Client,
    body: Option<&InProgressAuth>,
) -> Result<AuthChainResponse, SourceError> {
    let request = auth_headers(client.post(AUTHENTICATE_URL));

    let request = match body {
        Some(body) => request.json(body),
        None => request.body(""),
    };

    let response = request
        .send()
        .await
        .context("Failed to post o2 authenticate")?;
    let status = response.status();
    let body_text = response
        .text()
        .await
        .context("Failed to read o2 authenticate body")?;

    if status == StatusCode::UNAUTHORIZED {
        return Err(SourceError::InvalidCredentials {
            source_name: KIND.name().to_string(),
            message: "o2 authenticate returned 401".to_string(),
        });
    }

    if !status.is_success() {
        return Err(anyhow!(
            "o2 authenticate failed ({status}): {}",
            &body_text[..body_text.floor_char_boundary(500)]
        )
        .into());
    }

    serde_json::from_str(&body_text).map_err(|error| {
        let path = env::temp_dir().join("papermill-o2-decode-failure.json");
        let _ = fs::write(&path, &body_text);
        anyhow!(
            "Failed to decode o2 authenticate response: {error} (raw body written to {})",
            path.display()
        )
        .into()
    })
}

const MAX_AUTH_CHAIN_STEPS: usize = 12;

async fn walk_auth_chain(
    client: &reqwest::Client,
    credentials: &UsernamePasswordCredentials,
) -> Result<String, SourceError> {
    let mut response = post_authenticate(client, None).await?;
    let mut last_stage: Option<String> = None;

    for step in 0..MAX_AUTH_CHAIN_STEPS {
        match response {
            AuthChainResponse::Final(final_response) => {
                tracing::debug!(step, "o2 auth chain reached final stage");
                return Ok(final_response.token_id);
            }
            AuthChainResponse::InProgress(mut in_progress) => {
                let stage = in_progress.stage.clone().unwrap_or_default();
                tracing::debug!(step, %stage, "o2 auth chain step");
                last_stage = Some(stage);
                fill_stage(&mut in_progress, credentials)?;
                response = post_authenticate(client, Some(&in_progress)).await?;
            }
        }
    }

    Err(anyhow!(
        "o2 auth chain did not complete after {MAX_AUTH_CHAIN_STEPS} steps (last stage: {})",
        last_stage.as_deref().unwrap_or("<unknown>")
    )
    .into())
}

async fn establish_portal_session(
    client: &reqwest::Client,
    jar: &Arc<Jar>,
    token_id: &str,
) -> Result<(), SourceError> {
    let login_url =
        Url::parse(&format!("https://{LOGIN_HOST}/")).context("Failed to parse o2 login URL")?;

    jar.add_cookie_str(
        &format!("{CIAM_COOKIE_NAME}={token_id}; Path=/; Secure"),
        &login_url,
    );

    let response = client
        .get(PORTAL_AUTHENTICATE_URL)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .query(&[("clientId", OAUTH_CLIENT_ID), ("target-app", "")])
        .send()
        .await
        .context("Failed to call o2 portal authenticate")?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "o2 portal authenticate failed ({status}): {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    Ok(())
}

fn load_device_jwt(jar: &Arc<Jar>) -> anyhow::Result<()> {
    let Some(stored) = read_device_jwt()? else {
        return Ok(());
    };

    let login_url =
        Url::parse(&format!("https://{LOGIN_HOST}/")).context("Failed to parse o2 login URL")?;
    jar.add_cookie_str(
        &format!(
            "{DEVICE_JWT_COOKIE_NAME}={}; Path=/; Secure",
            stored.expose_secret()
        ),
        &login_url,
    );

    Ok(())
}

fn persist_device_jwt(jar: &Arc<Jar>) -> anyhow::Result<()> {
    let login_url =
        Url::parse(&format!("https://{LOGIN_HOST}/")).context("Failed to parse o2 login URL")?;

    let Some(header) = jar.cookies(&login_url) else {
        return Ok(());
    };

    let cookie_str = header
        .to_str()
        .context("Failed to parse cookie header as ASCII")?;

    for pair in cookie_str.split("; ") {
        let mut parts = pair.splitn(2, '=');

        if let (Some(name), Some(value)) = (parts.next(), parts.next())
            && name == DEVICE_JWT_COOKIE_NAME
        {
            write_device_jwt(&SecretString::from(value.to_string()))?;
            return Ok(());
        }
    }

    Ok(())
}

pub async fn authenticate(
    client: &reqwest::Client,
    jar: &Arc<Jar>,
    credentials: &UsernamePasswordCredentials,
) -> Result<(), SourceError> {
    load_device_jwt(jar)?;
    let token_id = walk_auth_chain(client, credentials).await?;
    establish_portal_session(client, jar, &token_id).await?;
    persist_device_jwt(jar)?;
    Ok(())
}
