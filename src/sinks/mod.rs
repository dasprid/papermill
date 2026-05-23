use std::borrow::Cow;
use std::str::FromStr;

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{EnumIter, IntoEnumIterator};

pub mod filesystem;
pub mod paperless;
pub mod wizard;

pub use filesystem::FilesystemSink;
pub use paperless::PaperlessSink;
pub use wizard::SinkWizard;

use crate::sources::{Invoice, InvoiceContent, SourceKind};

macro_rules! sinks {
    ($($variant:ident($name:literal, $label:literal)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
        pub enum SinkKind {
            $($variant,)*
        }

        impl SinkKind {
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            #[allow(dead_code)]
            pub fn label(&self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)*
                }
            }
        }

        impl FromStr for SinkKind {
            type Err = anyhow::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::iter()
                    .find(|kind| kind.name() == s)
                    .ok_or_else(|| anyhow!("unknown sink \"{s}\""))
            }
        }
    };
}

impl Serialize for SinkKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for SinkKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

sinks! {
    Paperless("paperless", "Paperless-ngx"),
    Filesystem("filesystem", "Filesystem"),
}

impl SinkKind {
    pub fn wizard(&self) -> Box<dyn SinkWizard> {
        match self {
            Self::Paperless => Box::new(paperless::PaperlessSinkWizard),
            Self::Filesystem => Box::new(filesystem::FilesystemSinkWizard),
        }
    }
}

pub struct DeliveryContext<'a> {
    pub source_kind: SourceKind,
    pub invoice: &'a Invoice,
    pub content: Cow<'a, InvoiceContent>,
}

#[derive(Debug)]
pub struct DeliveryReceipt {
    pub reference: Option<String>,
}

#[async_trait]
pub trait Sink: Send + Sync {
    #[allow(dead_code)]
    fn kind(&self) -> SinkKind;
    fn instance_name(&self) -> &str;
    async fn deliver(&self, ctx: DeliveryContext<'_>) -> anyhow::Result<DeliveryReceipt>;
}
