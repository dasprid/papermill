use async_trait::async_trait;
use chrono::NaiveDate;

use super::{Invoice, InvoiceContent, Source, SourceError, SourceKind};

pub struct MockSource;

impl MockSource {
    pub const KIND: SourceKind = SourceKind::Mock;

    pub async fn new() -> Result<Self, SourceError> {
        Ok(Self)
    }
}

#[async_trait]
impl Source for MockSource {
    fn kind(&self) -> SourceKind {
        Self::KIND
    }

    async fn list_invoices(
        &mut self,
        since: Option<NaiveDate>,
    ) -> Result<Vec<Invoice>, SourceError> {
        let invoice = Invoice {
            external_id: "MOCK-2026-001".to_string(),
            invoice_number: "MOCK-2026-001".to_string(),
            issued_on: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
        };

        if let Some(since) = since
            && invoice.issued_on < since
        {
            return Ok(vec![]);
        }

        Ok(vec![invoice])
    }

    async fn download_invoice(&mut self, invoice: &Invoice) -> Result<InvoiceContent, SourceError> {
        Ok(InvoiceContent {
            bytes: build_mock_pdf(),
            filename: format!("{}.pdf", invoice.invoice_number),
            content_type: "application/pdf".to_string(),
        })
    }
}

fn build_mock_pdf() -> Vec<u8> {
    let objects = [
        "<</Type /Catalog /Pages 2 0 R>>",
        "<</Type /Pages /Kids [3 0 R] /Count 1>>",
        "<</Type /Page /Parent 2 0 R /Resources <<>> /MediaBox [0 0 200 100]>>",
    ];

    let mut body = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());

    for (index, object) in objects.iter().enumerate() {
        offsets.push(body.len());
        body.push_str(&format!("{} 0 obj\n{}\nendobj\n", index + 1, object));
    }

    let xref_start = body.len();
    body.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));

    for offset in &offsets {
        body.push_str(&format!("{:010} 00000 n \n", offset));
    }

    body.push_str(&format!(
        "trailer\n<</Size {} /Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
        objects.len() + 1,
        xref_start
    ));

    body.into_bytes()
}
