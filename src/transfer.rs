use std::borrow::Cow;

use anyhow::anyhow;
use chrono::Utc;

use crate::shutdown;
use crate::sinks::{DeliveryContext, Sink};
use crate::sources::{Source, SourceError};
use crate::state::{StateStore, UploadRecord};

#[derive(Debug, Default)]
pub struct TransferOutcome {
    pub discovered: usize,
    pub uploaded: usize,
    pub skipped: usize,
}

pub enum SinkTarget {
    Live(Box<dyn Sink>),
    DryRun(String),
}

impl SinkTarget {
    pub fn instance_name(&self) -> &str {
        match self {
            Self::Live(sink) => sink.instance_name(),
            Self::DryRun(name) => name,
        }
    }
}

pub async fn run_transfer(
    source: &mut dyn Source,
    targets: &[SinkTarget],
    state: &StateStore,
    dry_run: bool,
) -> Result<TransferOutcome, SourceError> {
    if targets.is_empty() {
        return Err(anyhow!("run_transfer: no sink targets configured").into());
    }

    let since = state.last_issued_date(source.instance_name()).await?;
    let invoices = source.list_invoices(since).await?;

    let mut outcome = TransferOutcome {
        discovered: invoices.len(),
        ..Default::default()
    };

    for invoice in &invoices {
        if shutdown::is_shutting_down() {
            break;
        }

        let pending_targets =
            pending_for_invoice(state, source.instance_name(), invoice, targets).await?;

        if pending_targets.is_empty() {
            tracing::debug!(
                source = source.instance_name(),
                "skipping already-uploaded invoice {}",
                invoice.invoice_number
            );
            outcome.skipped += 1;
            continue;
        }

        if dry_run {
            for target in &pending_targets {
                tracing::info!(
                    source = source.instance_name(),
                    sink = target.instance_name(),
                    "dry-run upload: {} ({})",
                    invoice.invoice_number,
                    invoice.issued_on
                );
                outcome.uploaded += 1;
            }

            continue;
        }

        let content = source.download_invoice(invoice).await?;

        for target in pending_targets {
            let SinkTarget::Live(sink) = target else {
                continue;
            };

            tracing::info!(
                source = source.instance_name(),
                sink = sink.instance_name(),
                "delivering {} ({})",
                invoice.invoice_number,
                invoice.issued_on
            );

            let ctx = DeliveryContext {
                source_kind: source.kind(),
                invoice,
                content: Cow::Borrowed(&content),
            };
            let receipt = sink.deliver(ctx).await?;

            state
                .record_upload(&UploadRecord {
                    source_id: source.instance_name().to_string(),
                    sink_id: sink.instance_name().to_string(),
                    external_id: invoice.external_id.clone(),
                    sink_reference: receipt.reference,
                    invoice_issued_at: invoice.issued_on,
                    uploaded_at: Utc::now(),
                })
                .await?;

            outcome.uploaded += 1;
        }
    }

    Ok(outcome)
}

async fn pending_for_invoice<'a>(
    state: &StateStore,
    source_id: &str,
    invoice: &crate::sources::Invoice,
    targets: &'a [SinkTarget],
) -> Result<Vec<&'a SinkTarget>, SourceError> {
    let mut pending = Vec::with_capacity(targets.len());

    for target in targets {
        if !state
            .is_uploaded(source_id, target.instance_name(), &invoice.external_id)
            .await?
        {
            pending.push(target);
        }
    }

    Ok(pending)
}
