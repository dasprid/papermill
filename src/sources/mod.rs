use std::str::FromStr;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{EnumIter, IntoEnumIterator};
use thiserror::Error;

mod o2;
mod vodafone;
pub mod wizard;

pub use o2::O2Source;
pub use vodafone::VodafoneSource;
pub use wizard::{SourceWizard, UsernamePasswordSourceWizard};

macro_rules! sources {
    ($($variant:ident($ty:ty, $name:literal, $label:literal)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
        pub enum SourceKind {
            $($variant,)*
        }

        impl SourceKind {
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            pub fn label(&self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)*
                }
            }

            pub async fn build(&self, instance_name: &str) -> Result<Box<dyn Source>, SourceError> {
                match self {
                    $(Self::$variant => Ok(Box::new(<$ty>::new(instance_name).await?)),)*
                }
            }
        }

        impl FromStr for SourceKind {
            type Err = anyhow::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::iter()
                    .find(|kind| kind.name() == s)
                    .ok_or_else(|| anyhow!("unknown source \"{s}\""))
            }
        }

        $(
            const _: () = assert!(
                matches!(<$ty>::KIND, SourceKind::$variant),
                concat!(
                    "KIND on ", stringify!($ty),
                    " does not match SourceKind::", stringify!($variant),
                    " in the sources! macro invocation",
                ),
            );
        )*
    };
}

impl Serialize for SourceKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for SourceKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

sources! {
    Vodafone(VodafoneSource, "vodafone", "Vodafone"),
    O2(O2Source, "o2", "Telefónica O2"),
}

impl SourceKind {
    pub fn wizard(&self) -> Box<dyn SourceWizard> {
        match self {
            Self::Vodafone | Self::O2 => Box::new(UsernamePasswordSourceWizard::new(*self)),
        }
    }
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("invalid credentials for source \"{source_name}\": {message}")]
    InvalidCredentials {
        source_name: String,
        message: String,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct Invoice {
    pub external_id: String,
    pub invoice_number: String,
    pub issued_on: NaiveDate,
}

#[derive(Debug, Clone)]
pub struct InvoiceContent {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub content_type: String,
}

#[async_trait]
pub trait Source: Send {
    fn kind(&self) -> SourceKind;
    fn instance_name(&self) -> &str;
    async fn list_invoices(
        &mut self,
        since: Option<NaiveDate>,
    ) -> Result<Vec<Invoice>, SourceError>;
    async fn download_invoice(&mut self, invoice: &Invoice) -> Result<InvoiceContent, SourceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_parses_known_source() {
        let kind: SourceKind = "vodafone".parse().unwrap();
        assert!(matches!(kind, SourceKind::Vodafone));
    }

    #[test]
    fn from_str_rejects_unknown_source() {
        let result: Result<SourceKind, _> = "nonexistent".parse();
        assert!(result.is_err());
    }

    #[test]
    fn name_round_trips_through_from_str() {
        for kind in SourceKind::iter() {
            let parsed: SourceKind = kind.name().parse().unwrap();
            assert_eq!(parsed.name(), kind.name());
        }
    }
}
