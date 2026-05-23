use chrono::{NaiveDate, Utc};

use crate::shutdown;
use crate::sources::{Source, SourceError};
use crate::state::{StateStore, UploadRecord};

const MANUAL_TASK_REFERENCE: &str = "manually-uploaded";

#[derive(Debug, Default)]
pub struct MarkOutcome {
    pub discovered: usize,
    pub marked: usize,
    pub already_known: usize,
    pub skipped_by_until: usize,
}

pub async fn run_mark(
    source: &mut dyn Source,
    sink_ids: &[String],
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

        for sink_id in sink_ids {
            if state
                .is_uploaded(source.instance_name(), sink_id, &invoice.external_id)
                .await?
            {
                outcome.already_known += 1;
                continue;
            }

            if dry_run {
                tracing::info!(
                    source = source.instance_name(),
                    sink = sink_id.as_str(),
                    "dry-run mark: {} {}",
                    invoice.invoice_number,
                    invoice.issued_on
                );
                outcome.marked += 1;
                continue;
            }

            state
                .record_upload(&UploadRecord {
                    source_id: source.instance_name().to_string(),
                    sink_id: sink_id.clone(),
                    external_id: invoice.external_id.clone(),
                    sink_reference: Some(MANUAL_TASK_REFERENCE.to_string()),
                    invoice_issued_at: invoice.issued_on,
                    uploaded_at: now,
                })
                .await?;

            outcome.marked += 1;
        }
    }

    Ok(outcome)
}
