use anyhow::{Context, bail};
use inquire::{Password, PasswordDisplayMode};
use reqwest::Url;
use secrecy::SecretString;

use crate::{keystore, tty};

mod client;
mod wizard;

pub use client::{NamedItem, PaperlessClient, PaperlessSink};
pub use wizard::PaperlessSinkWizard;

pub fn read_token(instance_name: &str) -> anyhow::Result<Option<SecretString>> {
    keystore::read_secret(&keystore::sink_account(instance_name))
}

pub fn write_token(instance_name: &str, token: &SecretString) -> anyhow::Result<()> {
    keystore::write_secret(&keystore::sink_account(instance_name), token)
}

pub fn delete_token(instance_name: &str) -> anyhow::Result<bool> {
    keystore::delete(&keystore::sink_account(instance_name))
}

pub async fn verify(base_url: Url, token: SecretString) -> anyhow::Result<()> {
    let client = PaperlessClient::new(base_url, token)?;
    client.list_correspondents().await?;
    Ok(())
}

pub fn resolve_token(instance_name: &str) -> anyhow::Result<SecretString> {
    if let Some(token) = read_token(instance_name)? {
        return Ok(token);
    }

    tty::require()?;

    let raw = Password::new(&format!("[{instance_name}] Paperless API token:"))
        .with_display_mode(PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .context("Failed to read paperless token")?;

    if raw.is_empty() {
        bail!("paperless API token is required");
    }

    let token = SecretString::from(raw);
    write_token(instance_name, &token)?;
    Ok(token)
}
