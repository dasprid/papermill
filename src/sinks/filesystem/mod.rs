use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::sinks::{DeliveryContext, DeliveryReceipt, Sink, SinkKind};

mod wizard;

pub use wizard::FilesystemSinkWizard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
    #[serde(default)]
    pub group_by_year: bool,
    pub filename_template: String,
}

pub const DEFAULT_FILENAME_TEMPLATE: &str = "{date}_{number}.pdf";

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("filename template must contain {{number}}")]
    MissingNumber,
}

pub fn validate_filename_template(template: &str) -> Result<(), TemplateError> {
    if !template.contains("{number}") {
        return Err(TemplateError::MissingNumber);
    }

    Ok(())
}

pub fn verify(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("Failed to create or access {}", root.display()))?;

    let marker = root.join(".papermill-check");
    fs::write(&marker, b"papermill write check\n")
        .with_context(|| format!("Failed to write to {}", root.display()))?;
    fs::remove_file(&marker).ok();

    Ok(())
}

pub struct FilesystemSink {
    instance_name: String,
    root: PathBuf,
    binding: FilesystemBinding,
}

impl FilesystemSink {
    pub fn new(instance_name: &str, root: PathBuf, binding: FilesystemBinding) -> Self {
        Self {
            instance_name: instance_name.to_string(),
            root,
            binding,
        }
    }

    fn target_path(&self, ctx: &DeliveryContext<'_>) -> PathBuf {
        let mut dir = self.root.clone();

        if let Some(sub) = &self.binding.subdir {
            dir.push(sub);
        }

        if self.binding.group_by_year {
            dir.push(ctx.invoice.issued_on.year().to_string());
        }

        let filename = render_template(
            &self.binding.filename_template,
            ctx.invoice.issued_on,
            &ctx.invoice.invoice_number,
            ctx.source_kind.name(),
        );

        dir.join(filename)
    }
}

#[async_trait]
impl Sink for FilesystemSink {
    fn kind(&self) -> SinkKind {
        SinkKind::Filesystem
    }

    fn instance_name(&self) -> &str {
        &self.instance_name
    }

    async fn deliver(&self, ctx: DeliveryContext<'_>) -> anyhow::Result<DeliveryReceipt> {
        let path = self.target_path(&ctx);

        if path.exists() {
            tracing::warn!(
                sink = self.instance_name.as_str(),
                path = %path.display(),
                "filesystem target already exists; skipping write",
            );

            return Ok(DeliveryReceipt {
                reference: Some(path.display().to_string()),
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        fs::write(&path, &ctx.content.bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(DeliveryReceipt {
            reference: Some(path.display().to_string()),
        })
    }
}

fn render_template(template: &str, date: NaiveDate, number: &str, source: &str) -> String {
    template
        .replace("{date}", &date.to_string())
        .replace("{number}", &sanitize(number))
        .replace("{source}", &sanitize(source))
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_must_contain_number() {
        assert!(validate_filename_template("{date}_{number}.pdf").is_ok());
        assert!(validate_filename_template("{source}_{number}.pdf").is_ok());

        assert!(matches!(
            validate_filename_template("{date}.pdf"),
            Err(TemplateError::MissingNumber)
        ));
    }

    #[test]
    fn render_substitutes_tokens() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let rendered = render_template("{date}_{source}_{number}.pdf", date, "INV-001", "vodafone");

        assert_eq!(rendered, "2026-01-15_vodafone_INV-001.pdf");
    }

    #[test]
    fn render_sanitizes_unsafe_characters() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let rendered = render_template("{number}.pdf", date, "INV/2026-001", "x");

        assert_eq!(rendered, "INV_2026-001.pdf");
    }
}
