use anyhow::anyhow;
use chrono::Utc;

use crate::config::SourceConfig;
use crate::paperless::{PaperlessClient, UploadMetadata};
use crate::shutdown;
use crate::sources::{Source, SourceError};
use crate::state::{StateStore, UploadRecord};

#[derive(Debug, Default)]
pub struct TransferOutcome {
    pub discovered: usize,
    pub uploaded: usize,
    pub skipped: usize,
}

pub async fn run_transfer(
    source: &mut dyn Source,
    target: &SourceConfig,
    paperless: Option<&PaperlessClient>,
    state: &StateStore,
    dry_run: bool,
) -> Result<TransferOutcome, SourceError> {
    if !dry_run && paperless.is_none() {
        return Err(
            anyhow!("run_transfer: paperless client required when not in dry-run mode").into(),
        );
    }

    let since = state.last_issued_date(source.kind().name()).await?;
    let invoices = source.list_invoices(since).await?;

    let mut outcome = TransferOutcome {
        discovered: invoices.len(),
        ..Default::default()
    };

    for invoice in &invoices {
        if shutdown::is_shutting_down() {
            break;
        }

        if state
            .is_uploaded(source.kind().name(), &invoice.external_id)
            .await?
        {
            tracing::debug!(
                source = source.kind().name(),
                "skipping already-uploaded invoice {}",
                invoice.invoice_number
            );
            outcome.skipped += 1;
            continue;
        }

        if dry_run {
            tracing::info!(
                source = source.kind().name(),
                "dry-run upload: {} ({})",
                invoice.invoice_number,
                invoice.issued_on
            );
            outcome.uploaded += 1;
            continue;
        }

        tracing::info!(
            source = source.kind().name(),
            "transferring {} ({})",
            invoice.invoice_number,
            invoice.issued_on
        );

        let content = source.download_invoice(invoice).await?;
        let metadata = UploadMetadata {
            title: Some(format!(
                "{} {}",
                source.kind().name(),
                invoice.invoice_number
            )),
            created_on: Some(invoice.issued_on),
            correspondent_id: target.correspondent_id,
            document_type_id: target.document_type_id,
            tag_ids: target.tag_ids.clone(),
        };

        let client = paperless.expect("guarded above when !dry_run");
        let result = client.upload(content, &metadata).await?;

        state
            .record_upload(&UploadRecord {
                source_id: source.kind().name().to_string(),
                external_id: invoice.external_id.clone(),
                paperless_task_id: result.task_id,
                invoice_issued_at: invoice.issued_on,
                uploaded_at: Utc::now(),
            })
            .await?;

        outcome.uploaded += 1;
    }

    Ok(outcome)
}
