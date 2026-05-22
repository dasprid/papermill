use anyhow::{Context, anyhow};
use keyring_core::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, de::DeserializeOwned};

const SERVICE: &str = "papermill";

fn entry(account: &str) -> anyhow::Result<Entry> {
    Entry::new(SERVICE, account).context("Failed to construct keyring entry")
}

fn read_string(account: &str) -> anyhow::Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(anyhow!(error).context("Failed to read keyring entry")),
    }
}

pub fn read_secret(account: &str) -> anyhow::Result<Option<SecretString>> {
    Ok(read_string(account)?.map(SecretString::from))
}

pub fn write_secret(account: &str, value: &SecretString) -> anyhow::Result<()> {
    entry(account)?
        .set_password(value.expose_secret())
        .context("Failed to write keyring entry")?;
    Ok(())
}

pub fn read_stored<T: DeserializeOwned>(account: &str) -> anyhow::Result<Option<T>> {
    let Some(raw) = read_string(account)? else {
        return Ok(None);
    };

    serde_json::from_str(&raw)
        .map(Some)
        .context("Failed to parse stored credentials JSON")
}

pub fn write_stored<T: Serialize>(account: &str, value: &T) -> anyhow::Result<()> {
    let raw = serde_json::to_string(value).context("Failed to serialize credentials")?;
    entry(account)?
        .set_password(&raw)
        .context("Failed to write keyring entry")?;
    Ok(())
}

pub fn delete(account: &str) -> anyhow::Result<bool> {
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(KeyringError::NoEntry) => Ok(false),
        Err(error) => Err(anyhow!(error).context("Failed to delete keyring entry")),
    }
}
