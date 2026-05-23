use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use chrono::NaiveDate;
use reqwest::cookie::Jar;

use crate::credentials::{Credentials, UsernamePasswordCredentials};

use super::{Invoice, InvoiceContent, Source, SourceError, SourceKind};

mod auth;
mod callbacks;
mod client;
mod pow;
mod schemas;

struct DocumentRef {
    subscription_id: u64,
    bill_number: String,
    document_type: String,
}

pub struct O2Source {
    instance_name: String,
    http: reqwest::Client,
    subscription_ids: Vec<u64>,
    document_refs: HashMap<String, DocumentRef>,
}

impl O2Source {
    pub const KIND: SourceKind = SourceKind::O2;

    pub async fn new(instance_name: &str) -> Result<Self, SourceError> {
        let credentials = UsernamePasswordCredentials::resolve(instance_name)?;

        let jar = Arc::new(Jar::default());
        let http = crate::http::client_builder()
            .cookie_provider(jar.clone())
            .build()
            .context("Failed to build o2 HTTP client")?;

        auth::authenticate(&http, &jar, &credentials).await?;
        let subscription_ids = client::fetch_mobile_subscription_ids(&http).await?;

        Ok(Self {
            instance_name: instance_name.to_string(),
            http,
            subscription_ids,
            document_refs: HashMap::new(),
        })
    }
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[async_trait]
impl Source for O2Source {
    fn kind(&self) -> SourceKind {
        Self::KIND
    }

    fn instance_name(&self) -> &str {
        &self.instance_name
    }

    async fn list_invoices(
        &mut self,
        since: Option<NaiveDate>,
    ) -> Result<Vec<Invoice>, SourceError> {
        let mut result = Vec::new();

        for &subscription_id in &self.subscription_ids {
            let entries = client::fetch_invoice_list(&self.http, subscription_id).await?;

            for entry in entries {
                if let Some(since) = since
                    && entry.date < since
                {
                    continue;
                }

                let Some(document) = entry.bill_documents.into_iter().next() else {
                    continue;
                };

                self.document_refs.insert(
                    document.bill_number.clone(),
                    DocumentRef {
                        subscription_id,
                        bill_number: document.bill_number.clone(),
                        document_type: document.document_type.clone(),
                    },
                );

                result.push(Invoice {
                    external_id: document.bill_number.clone(),
                    invoice_number: document.bill_number,
                    issued_on: entry.date,
                });
            }
        }

        Ok(result)
    }

    async fn download_invoice(&mut self, invoice: &Invoice) -> Result<InvoiceContent, SourceError> {
        let reference = self
            .document_refs
            .get(&invoice.external_id)
            .with_context(|| {
                format!(
                    "Failed to find document reference for invoice {} (call list_invoices first)",
                    invoice.external_id
                )
            })?;

        let bytes = client::fetch_invoice_pdf(
            &self.http,
            reference.subscription_id,
            &reference.bill_number,
            &reference.document_type,
        )
        .await?;

        Ok(InvoiceContent {
            bytes,
            filename: format!("o2-{}.pdf", sanitize_filename(&invoice.invoice_number)),
            content_type: "application/pdf".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename;

    #[test]
    fn keeps_alphanumeric_and_allowed_punctuation() {
        assert_eq!(sanitize_filename("abc-123.def_ghi"), "abc-123.def_ghi");
    }

    #[test]
    fn replaces_path_separators_and_spaces() {
        assert_eq!(sanitize_filename("a/b\\c d"), "a_b_c_d");
    }

    #[test]
    fn preserves_unicode_letters() {
        assert_eq!(sanitize_filename("café"), "café");
    }
}
