use anyhow::{Context, bail};
use async_trait::async_trait;
use chrono::NaiveDate;
use reqwest::Url;
use reqwest::multipart::{Form, Part};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::config::PaperlessBinding;
use crate::sinks::{DeliveryContext, DeliveryReceipt, Sink, SinkKind};
use crate::sources::InvoiceContent;

pub struct PaperlessClient {
    base_url: Url,
    token: SecretString,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedItem {
    pub id: u32,
    pub name: String,
}

#[derive(Deserialize)]
struct PaperlessListPage<T> {
    results: Vec<T>,
}

#[derive(Default)]
pub struct UploadMetadata {
    pub title: Option<String>,
    pub created_on: Option<NaiveDate>,
    pub correspondent_id: Option<u32>,
    pub document_type_id: Option<u32>,
    pub tag_ids: Vec<u32>,
}

pub struct PaperlessUploadResult {
    pub task_id: String,
}

impl PaperlessClient {
    pub fn new(base_url: Url, token: SecretString) -> anyhow::Result<Self> {
        let http = crate::http::client_builder()
            .build()
            .context("Failed to build reqwest client")?;

        Ok(Self {
            base_url,
            token,
            http,
        })
    }

    pub async fn upload(
        &self,
        content: InvoiceContent,
        metadata: &UploadMetadata,
    ) -> anyhow::Result<PaperlessUploadResult> {
        let endpoint = self
            .base_url
            .join("api/documents/post_document/")
            .context("Failed to construct paperless upload URL")?;

        let document_part = Part::bytes(content.bytes)
            .file_name(content.filename)
            .mime_str(&content.content_type)
            .context("Failed to set document mime type")?;

        let mut form = Form::new().part("document", document_part);

        if let Some(title) = metadata.title.clone() {
            form = form.text("title", title);
        }

        if let Some(created) = metadata.created_on {
            form = form.text("created", created.to_string());
        }

        if let Some(id) = metadata.correspondent_id {
            form = form.text("correspondent", id.to_string());
        }

        if let Some(id) = metadata.document_type_id {
            form = form.text("document_type", id.to_string());
        }

        for tag_id in &metadata.tag_ids {
            form = form.text("tags", tag_id.to_string());
        }

        let response = self
            .http
            .post(endpoint)
            .header(
                "authorization",
                format!("Token {}", self.token.expose_secret()),
            )
            .multipart(form)
            .send()
            .await
            .context("Failed to post document to paperless")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "paperless upload failed ({status}): {}",
                &body[..body.floor_char_boundary(500)]
            );
        }

        let task_id: String = response
            .json()
            .await
            .context("Failed to decode paperless task UUID")?;

        Ok(PaperlessUploadResult { task_id })
    }

    pub async fn list_correspondents(&self) -> anyhow::Result<Vec<NamedItem>> {
        self.list_named("api/correspondents/").await
    }

    pub async fn list_document_types(&self) -> anyhow::Result<Vec<NamedItem>> {
        self.list_named("api/document_types/").await
    }

    pub async fn list_tags(&self) -> anyhow::Result<Vec<NamedItem>> {
        self.list_named("api/tags/").await
    }

    async fn list_named(&self, path: &str) -> anyhow::Result<Vec<NamedItem>> {
        let endpoint = self
            .base_url
            .join(path)
            .with_context(|| format!("Failed to construct paperless URL for {path}"))?;

        let response = self
            .http
            .get(endpoint)
            .header(
                "authorization",
                format!("Token {}", self.token.expose_secret()),
            )
            .query(&[("page_size", "1000")])
            .send()
            .await
            .with_context(|| format!("Failed to fetch {path} from paperless"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "paperless {path} failed ({status}): {}",
                &body[..body.floor_char_boundary(500)]
            );
        }

        let page: PaperlessListPage<NamedItem> = response
            .json()
            .await
            .with_context(|| format!("Failed to decode paperless {path} page"))?;

        Ok(page.results)
    }
}

pub struct PaperlessSink {
    instance_name: String,
    client: PaperlessClient,
    binding: PaperlessBinding,
}

impl PaperlessSink {
    pub fn new(instance_name: &str, client: PaperlessClient, binding: PaperlessBinding) -> Self {
        Self {
            instance_name: instance_name.to_string(),
            client,
            binding,
        }
    }
}

#[async_trait]
impl Sink for PaperlessSink {
    fn kind(&self) -> SinkKind {
        SinkKind::Paperless
    }

    fn instance_name(&self) -> &str {
        &self.instance_name
    }

    async fn deliver(&self, ctx: DeliveryContext<'_>) -> anyhow::Result<DeliveryReceipt> {
        let metadata = UploadMetadata {
            title: Some(format!(
                "{} {}",
                ctx.source_kind.name(),
                ctx.invoice.invoice_number
            )),
            created_on: Some(ctx.invoice.issued_on),
            correspondent_id: self.binding.correspondent_id,
            document_type_id: self.binding.document_type_id,
            tag_ids: self.binding.tag_ids.clone(),
        };

        let result = self
            .client
            .upload(ctx.content.into_owned(), &metadata)
            .await?;
        Ok(DeliveryReceipt {
            reference: Some(result.task_id),
        })
    }
}
