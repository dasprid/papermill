use std::collections::HashMap;

use anyhow::Context;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::NaiveDate;
use reqwest::redirect::Policy;
use secrecy::SecretString;

use crate::credentials::{Credentials, UsernamePasswordCredentials};

use super::{Invoice, InvoiceContent, Source, SourceError, SourceKind};

mod auth;
mod client;
mod schemas;

struct DocumentRef {
    customer_urn: String,
    document_id: String,
}

pub struct VodafoneSource {
    instance_name: String,
    http: reqwest::Client,
    access_token: SecretString,
    customer_urns: Vec<String>,
    document_refs: HashMap<String, DocumentRef>,
}

impl VodafoneSource {
    pub const KIND: SourceKind = SourceKind::Vodafone;

    pub async fn new(instance_name: &str) -> Result<Self, SourceError> {
        let credentials = UsernamePasswordCredentials::resolve(instance_name)?;

        let http = crate::http::client_builder()
            .cookie_store(true)
            .redirect(Policy::none())
            .build()
            .context("Failed to build vodafone HTTP client")?;

        let access_token = auth::authenticate(&http, &credentials).await?;
        let customer_urns = client::fetch_customer_urns(&http, &access_token).await?;

        Ok(Self {
            instance_name: instance_name.to_string(),
            http,
            access_token,
            customer_urns,
            document_refs: HashMap::new(),
        })
    }
}

#[async_trait]
impl Source for VodafoneSource {
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

        for urn in &self.customer_urns {
            let entries = client::fetch_invoice_list(&self.http, &self.access_token, urn).await?;

            for entry in entries {
                if let Some(since) = since
                    && entry.date < since
                {
                    continue;
                }

                let Some(document) = entry.documents.iter().find(|d| d.category == "invoice")
                else {
                    continue;
                };

                self.document_refs.insert(
                    entry.number.clone(),
                    DocumentRef {
                        customer_urn: urn.clone(),
                        document_id: document.document_id.clone(),
                    },
                );

                result.push(Invoice {
                    external_id: entry.number.clone(),
                    invoice_number: entry.number,
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

        let document = client::fetch_invoice_document(
            &self.http,
            &self.access_token,
            &reference.customer_urn,
            &reference.document_id,
        )
        .await?;

        let bytes = BASE64
            .decode(document.data.as_bytes())
            .context("Failed to decode base64 PDF data")?;

        Ok(InvoiceContent {
            bytes,
            filename: format!("{}-{}.pdf", Self::KIND.name(), invoice.invoice_number),
            content_type: document.mime,
        })
    }
}
