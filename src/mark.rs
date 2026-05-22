use chrono::{NaiveDate, Utc};

use crate::shutdown;
use crate::sources::{Source, SourceError};
use crate::state::{StateStore, UploadRecord};

const MANUAL_TASK_ID: &str = "manually-uploaded";

#[derive(Debug, Default)]
pub struct MarkOutcome {
    pub discovered: usize,
    pub marked: usize,
    pub already_known: usize,
    pub skipped_by_until: usize,
}

pub async fn run_mark(
    source: &mut dyn Source,
    state: &StateStore,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> Result<MarkOutcome, SourceError> {
    let invoices = source.list_invoices(None).await?;
    let now = Utc::now();

    let mut outcome = MarkOutcome {
        discovered: invoices.len(),
        ..Default::default()
    };

    for invoice in &invoices {
        if shutdown::is_shutting_down() {
            break;
        }

        if let Some(until) = until
            && invoice.issued_on > until
        {
            outcome.skipped_by_until += 1;
            continue;
        }

        if state
            .is_uploaded(source.kind().name(), &invoice.external_id)
            .await?
        {
            outcome.already_known += 1;
            continue;
        }

        if dry_run {
            tracing::info!(
                source = source.kind().name(),
                "dry-run mark: {} {}",
                invoice.invoice_number,
                invoice.issued_on
            );
            outcome.marked += 1;
            continue;
        }

        state
            .record_upload(&UploadRecord {
                source_id: source.kind().name().to_string(),
                external_id: invoice.external_id.clone(),
                paperless_task_id: MANUAL_TASK_ID.to_string(),
                invoice_issued_at: invoice.issued_on,
                uploaded_at: now,
            })
            .await?;

        outcome.marked += 1;
    }

    Ok(outcome)
}
