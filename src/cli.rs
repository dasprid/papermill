use anyhow::{Context, bail};
use chrono::NaiveDate;
use clap::{ArgGroup, Parser, Subcommand};

use crate::config::{self, Config, SourceConfig};
use crate::credentials::delete_source_credentials;
use crate::mark::{MarkOutcome, run_mark};
use crate::paperless::{PaperlessClient, resolve_token};
use crate::shutdown;
use crate::sources::{SourceError, SourceKind};
use crate::state::StateStore;
use crate::transfer::{TransferOutcome, run_transfer};

#[derive(Parser)]
#[command(name = "papermill", version, about = env!("CARGO_PKG_DESCRIPTION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transfer new invoices to Paperless.
    #[command(group(ArgGroup::new("transfer-target").required(true).args(["source", "all"])))]
    Transfer {
        /// Source to transfer from.
        source: Option<SourceKind>,

        /// Transfer from every configured source.
        #[arg(long)]
        all: bool,

        #[arg(long)]
        dry_run: bool,
    },
    /// Mark invoices as already transferred without uploading.
    #[command(group(ArgGroup::new("mark-target").required(true).args(["source", "all"])))]
    Mark {
        /// Source to mark.
        source: Option<SourceKind>,

        /// Mark across every configured source.
        #[arg(long)]
        all: bool,

        #[arg(long)]
        until: Option<NaiveDate>,

        #[arg(long)]
        dry_run: bool,
    },
    /// Run the interactive setup wizard.
    Setup,
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Setup => crate::setup::run().await,
        Command::Transfer {
            source,
            all,
            dry_run,
        } => {
            if all {
                execute_transfer_all(dry_run).await
            } else {
                execute_transfer_one(source.expect("clap enforces source xor --all"), dry_run).await
            }
        }
        Command::Mark {
            source,
            all,
            until,
            dry_run,
        } => {
            if all {
                execute_mark_all(until, dry_run).await
            } else {
                execute_mark_one(
                    source.expect("clap enforces source xor --all"),
                    until,
                    dry_run,
                )
                .await
            }
        }
    }
}

fn print_transfer_outcome(kind: SourceKind, outcome: &TransferOutcome, dry_run: bool) {
    let suffix = if dry_run { " dry-run" } else { "" };
    println!(
        "[{}{suffix}] discovered={} uploaded={} skipped={}",
        kind.name(),
        outcome.discovered,
        outcome.uploaded,
        outcome.skipped,
    );
}

fn print_mark_outcome(kind: SourceKind, outcome: &MarkOutcome, dry_run: bool) {
    let suffix = if dry_run { " dry-run" } else { "" };
    println!(
        "[{} mark{suffix}] discovered={} marked={} already_known={} skipped_by_until={}",
        kind.name(),
        outcome.discovered,
        outcome.marked,
        outcome.already_known,
        outcome.skipped_by_until,
    );
}

async fn transfer_attempt(
    kind: SourceKind,
    target: &SourceConfig,
    paperless: Option<&PaperlessClient>,
    state: &StateStore,
    dry_run: bool,
) -> Result<TransferOutcome, SourceError> {
    let mut source = kind.build().await?;
    run_transfer(source.as_mut(), target, paperless, state, dry_run).await
}

async fn mark_attempt(
    kind: SourceKind,
    state: &StateStore,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> Result<MarkOutcome, SourceError> {
    let mut source = kind.build().await?;
    run_mark(source.as_mut(), state, until, dry_run).await
}

/// Run `operation` and, on `SourceError::InvalidCredentials`, delete the keyring entry for
/// `kind`, prompt for replacements, and retry exactly once. Any error after the retry propagates.
async fn with_credential_retry<F, Fut, T>(
    kind: SourceKind,
    mut operation: F,
) -> Result<T, SourceError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SourceError>>,
{
    match operation().await {
        Ok(value) => return Ok(value),
        Err(SourceError::InvalidCredentials { .. }) => {
            delete_source_credentials(kind.name()).ok();
            eprintln!(
                "[{}] credentials rejected, re-prompting and retrying once",
                kind.name()
            );
        }
        Err(other) => return Err(other),
    }

    operation().await
}

async fn execute_transfer_one(kind: SourceKind, dry_run: bool) -> anyhow::Result<()> {
    let config = config::load()?;

    let target = config.sources.get(&kind).with_context(|| {
        format!(
            "Failed to find target config for source \"{}\"",
            kind.name()
        )
    })?;

    let state = StateStore::open(&config::state_path()?).await?;
    let paperless = build_paperless_if_needed(&config, dry_run)?;

    let outcome = with_credential_retry(kind, || {
        transfer_attempt(kind, target, paperless.as_ref(), &state, dry_run)
    })
    .await?;

    print_transfer_outcome(kind, &outcome, dry_run);
    Ok(())
}

async fn execute_transfer_all(dry_run: bool) -> anyhow::Result<()> {
    let config = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;
    let paperless = build_paperless_if_needed(&config, dry_run)?;

    let kinds = sorted_source_kinds(&config);

    if kinds.is_empty() {
        bail!("Failed to find any configured sources");
    }

    let mut failed: Vec<SourceKind> = Vec::new();

    for kind in kinds {
        if shutdown::is_shutting_down() {
            break;
        }

        let target = config
            .sources
            .get(&kind)
            .expect("kind came from config.sources keys");

        let result = with_credential_retry(kind, || {
            transfer_attempt(kind, target, paperless.as_ref(), &state, dry_run)
        })
        .await;

        match result {
            Ok(outcome) => print_transfer_outcome(kind, &outcome, dry_run),
            Err(error) => {
                log_source_error(kind, &error);
                failed.push(kind);
            }
        }
    }

    if !failed.is_empty() {
        bail!("Failed to transfer: {}", join_kind_names(&failed));
    }

    Ok(())
}

async fn execute_mark_one(
    kind: SourceKind,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let _ = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;

    let outcome =
        with_credential_retry(kind, || mark_attempt(kind, &state, until, dry_run)).await?;

    print_mark_outcome(kind, &outcome, dry_run);
    Ok(())
}

async fn execute_mark_all(until: Option<NaiveDate>, dry_run: bool) -> anyhow::Result<()> {
    let config = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;

    let kinds = sorted_source_kinds(&config);

    if kinds.is_empty() {
        bail!("Failed to find any configured sources");
    }

    let mut failed: Vec<SourceKind> = Vec::new();

    for kind in kinds {
        if shutdown::is_shutting_down() {
            break;
        }

        let result =
            with_credential_retry(kind, || mark_attempt(kind, &state, until, dry_run)).await;

        match result {
            Ok(outcome) => print_mark_outcome(kind, &outcome, dry_run),
            Err(error) => {
                log_source_error(kind, &error);
                failed.push(kind);
            }
        }
    }

    if !failed.is_empty() {
        bail!("Failed to mark: {}", join_kind_names(&failed));
    }

    Ok(())
}

fn build_paperless_if_needed(
    config: &Config,
    dry_run: bool,
) -> anyhow::Result<Option<PaperlessClient>> {
    if dry_run {
        return Ok(None);
    }

    let token = resolve_token()?;
    let client = PaperlessClient::new(config.paperless.base_url.clone(), token)?;
    Ok(Some(client))
}

fn sorted_source_kinds(config: &Config) -> Vec<SourceKind> {
    let mut kinds: Vec<SourceKind> = config.sources.keys().copied().collect();
    kinds.sort_by_key(|k| k.name());
    kinds
}

fn join_kind_names(kinds: &[SourceKind]) -> String {
    kinds
        .iter()
        .map(|k| k.name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn log_source_error(kind: SourceKind, error: &SourceError) {
    let name = kind.name();
    match error {
        SourceError::InvalidCredentials { .. } => {
            eprintln!("[{name}] credentials still rejected after retry, keyring entry cleared");
        }
        SourceError::Other(inner) => {
            eprintln!("[{name}] failed: {inner:?}");
        }
    }
}
