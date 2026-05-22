use anyhow::{Context, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::credentials::UsernamePasswordCredentials;
use crate::sources::{SourceError, SourceKind};

use super::schemas::TokenResponse;

const KIND: SourceKind = SourceKind::Vodafone;
const CLIENT_ID: &str = "b0595a44-0726-11ec-9011-9457a55a403c";
const REDIRECT_URI: &str = "https://www.vodafone.de/meinvodafone/services/";
const SCOPE: &str = "openid profile webseal user-groups user-accounts validate-token update-email-username account user-data user-subscriptions";
const SESSION_START_URL: &str = "https://www.vodafone.de/mint/rest/v60/session/start";
const AUTHORIZE_URL: &str = "https://www.vodafone.de/mint/oidc/authorize";
const TOKEN_URL: &str = "https://www.vodafone.de/mint/oidc/token";

struct PkcePair {
    verifier: String,
    challenge: String,
}

fn generate_pkce_pair() -> PkcePair {
    let verifier_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge_bytes = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);

    PkcePair {
        verifier,
        challenge,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionStartBody<'a> {
    authn_identifier: &'a str,
    credential: &'a str,
}

async fn start_session(
    client: &reqwest::Client,
    credentials: &UsernamePasswordCredentials,
) -> Result<(), SourceError> {
    let body = SessionStartBody {
        authn_identifier: &credentials.username,
        credential: credentials.password.expose_secret(),
    };

    let response = client
        .post(SESSION_START_URL)
        .json(&body)
        .send()
        .await
        .context("Failed to post vodafone session/start")?;

    let status = response.status();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SourceError::InvalidCredentials {
            source_name: KIND.name().to_string(),
            message: format!("session/start rejected credentials ({status})"),
        });
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "vodafone session/start failed ({status}): {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    Ok(())
}

async fn request_authorization_code(
    client: &reqwest::Client,
    challenge: &str,
) -> Result<String, SourceError> {
    let response = client
        .get(AUTHORIZE_URL)
        .query(&[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
            ("redirect_uri", REDIRECT_URI),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("prompt", "none"),
        ])
        .send()
        .await
        .context("Failed to call vodafone authorize endpoint")?;

    let status = response.status();

    if !status.is_redirection() {
        return Err(anyhow!("vodafone authorize did not redirect: {status}").into());
    }

    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .context("Failed to find Location header on authorize 302")?
        .to_str()
        .context("Failed to parse Location header as ASCII")?;

    let redirected = Url::parse(location).context("Failed to parse authorize Location URL")?;

    if let Some((_, error)) = redirected.query_pairs().find(|(k, _)| k == "error") {
        let error = error.into_owned();

        if error == "login_required" || error == "access_denied" {
            return Err(SourceError::InvalidCredentials {
                source_name: KIND.name().to_string(),
                message: format!("authorize returned: {error}"),
            });
        }

        return Err(anyhow!("vodafone authorize returned error: {error}").into());
    }

    let code = redirected
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, value)| value.into_owned())
        .context("Failed to find 'code' parameter on authorize redirect")?;

    Ok(code)
}

async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<SecretString, SourceError> {
    let response = client
        .post(TOKEN_URL)
        .query(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .context("Failed to post vodafone token exchange")?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "vodafone token exchange failed ({status}): {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    let parsed: TokenResponse = response
        .json()
        .await
        .context("Failed to decode token response")?;
    Ok(SecretString::from(parsed.access_token))
}

pub async fn authenticate(
    client: &reqwest::Client,
    credentials: &UsernamePasswordCredentials,
) -> Result<SecretString, SourceError> {
    let pkce = generate_pkce_pair();
    start_session(client, credentials).await?;
    let code = request_authorization_code(client, &pkce.challenge).await?;
    let access_token = exchange_code(client, &code, &pkce.verifier).await?;
    Ok(access_token)
}
