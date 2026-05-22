use chrono::NaiveDate;
use serde::{Deserialize, Deserializer};

fn deserialize_date_triple<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
where
    D: Deserializer<'de>,
{
    let [year, month, day]: [i32; 3] = Deserialize::deserialize(deserializer)?;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
        .ok_or_else(|| serde::de::Error::custom(format!("invalid date [{year}, {month}, {day}]")))
}

#[derive(Deserialize)]
pub struct InvoiceListResponse {
    pub invoices: Vec<InvoiceListEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceListEntry {
    #[serde(deserialize_with = "deserialize_date_triple")]
    pub date: NaiveDate,
    #[serde(default)]
    pub bill_documents: Vec<BillDocument>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BillDocument {
    pub bill_number: String,
    pub document_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupInfo {
    pub customer_data: CustomerData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerData {
    pub customer_info: CustomerInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerInfo {
    #[serde(default)]
    pub account_infos: Vec<AccountInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(default)]
    pub subscription_infos: Vec<SubscriptionInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInfo {
    pub subscription_id: u64,
    pub subscription_type: String,
    pub active: bool,
}
