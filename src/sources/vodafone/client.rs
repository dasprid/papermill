use std::collections::BTreeSet;

use anyhow::{Context, anyhow};
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

use crate::sources::{SourceError, SourceKind};

use super::schemas::{
    InvoiceDocumentResponse, InvoiceListEntry, InvoiceListResponse, UserInfoEntry,
};

const KIND: SourceKind = SourceKind::Vodafone;
const API_BASE: &str = "https://api.vodafone.de/meinvodafone/v2";
const API_KEY: &str = "aEIoMCae0A933wBL0bLlS6SwSBfkKwM5";
const CLIENT_NAME: &str = "MyVFWeb";

fn to_short_urn(long: &str) -> String {
    long.replace("urn:vf-de-dxl-tmf:um:", "urn:vf-de:")
}

async fn call_api(
    client: &reqwest::Client,
    url: &str,
    access_token: &SecretString,
) -> Result<reqwest::Response, SourceError> {
    let response = client
        .get(url)
        .header(
            "authorization",
            format!("Bearer {}", access_token.expose_secret()),
        )
        .header("x-api-key", API_KEY)
        .header("x-vf-clientid", CLIENT_NAME)
        .header("accept", "application/json")
        .send()
        .await
        .context("Failed to call vodafone API")?;

    let status = response.status();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(SourceError::InvalidCredentials {
            source_name: KIND.name().to_string(),
            message: format!("API rejected token ({status})"),
        });
    }

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "vodafone API {status} at {url}: {}",
            &body[..body.floor_char_boundary(500)]
        )
        .into());
    }

    Ok(response)
}

pub async fn fetch_customer_urns(
    client: &reqwest::Client,
    access_token: &SecretString,
) -> Result<Vec<String>, SourceError> {
    let url = format!("{API_BASE}/tmf-api/openid/v4/userinfo");
    let response = call_api(client, &url, access_token).await?;
    let entries: Vec<UserInfoEntry> = response.json().await.context("Failed to decode userinfo")?;

    let mut urns: BTreeSet<String> = BTreeSet::new();

    for entry in &entries {
        for asset in &entry.user_assets {
            for related in &asset.related_asset {
                if related.entity_type == "billingAccount" {
                    urns.insert(to_short_urn(&related.id));
                }
            }
        }
    }

    if urns.is_empty() {
        return Err(anyhow!("vodafone userinfo returned no billing-account URNs").into());
    }

    Ok(urns.into_iter().collect())
}

pub async fn fetch_invoice_list(
    client: &reqwest::Client,
    access_token: &SecretString,
    customer_urn: &str,
) -> Result<Vec<InvoiceListEntry>, SourceError> {
    let url = format!("{API_BASE}/customer/{customer_urn}/invoice");
    let response = call_api(client, &url, access_token).await?;
    let parsed: InvoiceListResponse = response
        .json()
        .await
        .context("Failed to decode invoice list")?;
    Ok(parsed.invoices)
}

pub async fn fetch_invoice_document(
    client: &reqwest::Client,
    access_token: &SecretString,
    customer_urn: &str,
    document_id: &str,
) -> Result<InvoiceDocumentResponse, SourceError> {
    let url = format!("{API_BASE}/customer/{customer_urn}/invoiceDocument/{document_id}");
    let response = call_api(client, &url, access_token).await?;
    Ok(response
        .json()
        .await
        .context("Failed to decode invoice document")?)
}
