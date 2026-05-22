use anyhow::Context;
use inquire::{Password, PasswordDisplayMode, Text};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{keystore, tty};

const SOURCE_ACCOUNT_PREFIX: &str = "source:";

fn source_account(name: &str) -> String {
    format!("{SOURCE_ACCOUNT_PREFIX}{name}")
}

mod secret_serde {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        secret: &SecretString,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        secret.expose_secret().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SecretString, D::Error> {
        Ok(SecretString::from(String::deserialize(deserializer)?))
    }
}

pub trait Credentials: Sized + Serialize + DeserializeOwned {
    fn prompt(source_name: &str) -> anyhow::Result<Self>;

    fn resolve(source_name: &str) -> anyhow::Result<Self> {
        let account = source_account(source_name);

        if let Some(stored) = keystore::read_stored::<Self>(&account)? {
            return Ok(stored);
        }

        let credentials = Self::prompt(source_name)?;
        keystore::write_stored(&account, &credentials)?;
        Ok(credentials)
    }
}

#[derive(Serialize, Deserialize)]
pub struct UsernamePasswordCredentials {
    pub username: String,
    #[serde(with = "secret_serde")]
    pub password: SecretString,
}

impl Credentials for UsernamePasswordCredentials {
    fn prompt(source_name: &str) -> anyhow::Result<Self> {
        tty::require()?;

        let username = Text::new(&format!("[{source_name}] username:"))
            .prompt()
            .context("Failed to read username")?;

        let password_raw = Password::new(&format!("[{source_name}] password:"))
            .with_display_mode(PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt()
            .context("Failed to read password")?;

        Ok(Self {
            username,
            password: SecretString::from(password_raw),
        })
    }
}

pub fn delete_source_credentials(source_name: &str) -> anyhow::Result<bool> {
    keystore::delete(&source_account(source_name))
}
