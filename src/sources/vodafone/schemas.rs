use chrono::NaiveDate;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfoEntry {
    #[serde(default)]
    pub user_assets: Vec<UserAsset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAsset {
    #[serde(default)]
    pub related_asset: Vec<RelatedAsset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedAsset {
    pub id: String,
    pub entity_type: String,
}

#[derive(Deserialize)]
pub struct InvoiceListResponse {
    pub invoices: Vec<InvoiceListEntry>,
}

#[derive(Deserialize)]
pub struct InvoiceListEntry {
    pub number: String,
    pub date: NaiveDate,
    #[serde(default)]
    pub documents: Vec<InvoiceDocumentRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceDocumentRef {
    pub document_id: String,
    pub category: String,
}

#[derive(Deserialize)]
pub struct InvoiceDocumentResponse {
    pub mime: String,
    pub data: String,
}
