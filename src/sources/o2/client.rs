use std::env;
use std::fs;

use anyhow::{Context, anyhow};
use reqwest::StatusCode;
use url::Url;

use crate::sources::{SourceError, SourceKind};

use super::schemas::{InvoiceListEntry, InvoiceListResponse, StartupInfo};

const KIND: SourceKind = SourceKind::O2;
const API_BASE: &str = "https://www.o2online.de";

async fn call_json_api<T>(
    client: &reqwest::Client,
    url: &str,
    subscription_id: Option<u64>,
) -> Result<T, SourceError>
where
    T: serde::de::DeserializeOwned,
{
    let mut request = client
        .get(url)
        .header("accept", "application/json, text/plain, */*")
        .header("referer", "https://www.o2online.de/mein-o2/ecare/");

    if let Some(id) = subscription_id {
        request = request.header("ecareng-selected-subscription", id.to_string());
    }

    let response = request.send().await.context("Failed to call o2 JSON API")?;
    let status = response.status();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or("<binary>").to_string(),
                )
            })
            .collect();
        let body = response.text().await.unwrap_or_default();
        let preview = &body[..body.floor_char_boundary(300)];
        tracing::warn!(%url, %status, ?headers, body_preview = %preview, "o2 API rejected session");
        return Err(SourceError::InvalidCredentials {
            source_name: KIND.name().to_string(),
            message: format!("o2 API {status} at {url}: {preview}"),
        });
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "o2 API {status} at {url}: {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    let body_text = response
        .text()
        .await
        .context("Failed to read o2 JSON response body")?;
    let value: serde_json::Value = serde_json::from_str(&body_text).map_err(|error| {
        let path = env::temp_dir().join("papermill-o2-json-decode-failure.json");
        let _ = fs::write(&path, &body_text);
        anyhow!(
            "Failed to parse o2 JSON response from {url}: {error} (raw body written to {})",
            path.display()
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        let path = env::temp_dir().join("papermill-o2-json-decode-failure.json");
        let _ = fs::write(&path, &body_text);
        anyhow!(
            "Failed to decode o2 JSON response from {url}: {error} (raw body written to {})",
            path.display()
        )
        .into()
    })
}

pub async fn fetch_mobile_subscription_ids(
    client: &reqwest::Client,
) -> Result<Vec<u64>, SourceError> {
    let url = format!("{API_BASE}/ecareng/api/startupinfo");
    let info: StartupInfo = call_json_api(client, &url, None).await?;

    let mut ids = Vec::new();

    for account in &info.customer_data.customer_info.account_infos {
        for subscription in &account.subscription_infos {
            if subscription.subscription_type == "MOBILE" && subscription.active {
                ids.push(subscription.subscription_id);
            }
        }
    }

    if ids.is_empty() {
        return Err(anyhow!("o2 startupinfo returned no active MOBILE subscriptions").into());
    }

    Ok(ids)
}

pub async fn fetch_invoice_list(
    client: &reqwest::Client,
    subscription_id: u64,
) -> Result<Vec<InvoiceListEntry>, SourceError> {
    let url = format!("{API_BASE}/vt-billing/api/invoiceinfo");
    let response: InvoiceListResponse = call_json_api(client, &url, Some(subscription_id)).await?;
    Ok(response.invoices)
}

pub async fn fetch_invoice_pdf(
    client: &reqwest::Client,
    subscription_id: u64,
    bill_number: &str,
    document_type: &str,
) -> Result<Vec<u8>, SourceError> {
    let url = Url::parse_with_params(
        &format!("{API_BASE}/vt-billing/api/billdocument"),
        &[("billNumber", bill_number), ("documentType", document_type)],
    )
    .context("Failed to construct o2 billdocument URL")?;

    let response = client
        .get(url)
        .header("accept", "application/pdf")
        .header("ecareng-selected-subscription", subscription_id.to_string())
        .send()
        .await
        .context("Failed to download o2 PDF")?;

    let status = response.status();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SourceError::InvalidCredentials {
            source_name: KIND.name().to_string(),
            message: format!("o2 billdocument rejected session ({status})"),
        });
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "o2 billdocument {status}: {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    Ok(response
        .bytes()
        .await
        .context("Failed to read PDF bytes")?
        .to_vec())
}
