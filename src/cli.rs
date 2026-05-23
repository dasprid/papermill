use anyhow::{Context, bail};
use chrono::NaiveDate;
use clap::{ArgGroup, Parser, Subcommand};

use crate::config::{self, Config, SinkBinding, SinkInstance};
use crate::credentials::delete_source_credentials;
use crate::mark::{MarkOutcome, run_mark};
use crate::shutdown;
use crate::sinks::paperless::{PaperlessClient, resolve_token};
use crate::sinks::{FilesystemSink, PaperlessSink, Sink};
use crate::sources::{SourceError, SourceKind};
use crate::state::StateStore;
use crate::transfer::{SinkTarget, TransferOutcome, run_transfer};

#[derive(Parser)]
#[command(name = "papermill", version, about = env!("CARGO_PKG_DESCRIPTION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transfer new invoices to Paperless.
    #[command(group(
        ArgGroup::new("transfer-target")
            .required(true)
            .args(["source", "all", "kind"]),
    ))]
    Transfer {
        /// Source instance to transfer from.
        source: Option<String>,

        /// Transfer from every configured source instance.
        #[arg(long)]
        all: bool,

        /// Transfer from every configured instance of this source kind.
        #[arg(long, value_name = "KIND")]
        kind: Option<SourceKind>,

        #[arg(long)]
        dry_run: bool,
    },
    /// Mark invoices as already transferred without uploading.
    #[command(group(
        ArgGroup::new("mark-target")
            .required(true)
            .args(["source", "all", "kind"]),
    ))]
    Mark {
        /// Source instance to mark.
        source: Option<String>,

        /// Mark across every configured source instance.
        #[arg(long)]
        all: bool,

        /// Mark across every configured instance of this source kind.
        #[arg(long, value_name = "KIND")]
        kind: Option<SourceKind>,

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
            kind,
            dry_run,
        } => {
            if all {
                execute_transfer_many(SourceFilter::All, dry_run).await
            } else if let Some(kind) = kind {
                execute_transfer_many(SourceFilter::Kind(kind), dry_run).await
            } else {
                execute_transfer_one(
                    &source.expect("clap enforces source xor --all xor --kind"),
                    dry_run,
                )
                .await
            }
        }
        Command::Mark {
            source,
            all,
            kind,
            until,
            dry_run,
        } => {
            if all {
                execute_mark_many(SourceFilter::All, until, dry_run).await
            } else if let Some(kind) = kind {
                execute_mark_many(SourceFilter::Kind(kind), until, dry_run).await
            } else {
                execute_mark_one(
                    &source.expect("clap enforces source xor --all xor --kind"),
                    until,
                    dry_run,
                )
                .await
            }
        }
    }
}

enum SourceFilter {
    All,
    Kind(SourceKind),
}

fn instance_label(instance_name: &str, kind: SourceKind) -> String {
    if instance_name == kind.name() {
        instance_name.to_string()
    } else {
        format!("{instance_name} ({})", kind.name())
    }
}

fn print_transfer_outcome(label: &str, outcome: &TransferOutcome, dry_run: bool) {
    let suffix = if dry_run { " dry-run" } else { "" };
    println!(
        "[{label}{suffix}] discovered={} uploaded={} skipped={}",
        outcome.discovered, outcome.uploaded, outcome.skipped,
    );
}

fn print_mark_outcome(label: &str, outcome: &MarkOutcome, dry_run: bool) {
    let suffix = if dry_run { " dry-run" } else { "" };
    println!(
        "[{label} mark{suffix}] discovered={} marked={} already_known={} skipped_by_until={}",
        outcome.discovered, outcome.marked, outcome.already_known, outcome.skipped_by_until,
    );
}

async fn transfer_attempt(
    instance_name: &str,
    kind: SourceKind,
    targets: &[SinkTarget],
    state: &StateStore,
    dry_run: bool,
) -> Result<TransferOutcome, SourceError> {
    let mut source = kind.build(instance_name).await?;
    run_transfer(source.as_mut(), targets, state, dry_run).await
}

async fn mark_attempt(
    instance_name: &str,
    kind: SourceKind,
    sink_ids: &[String],
    state: &StateStore,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> Result<MarkOutcome, SourceError> {
    let mut source = kind.build(instance_name).await?;
    run_mark(source.as_mut(), sink_ids, state, until, dry_run).await
}

/// Run `operation` and, on `SourceError::InvalidCredentials`, delete the keyring entry for
/// `instance_name`, prompt for replacements, and retry exactly once.
async fn with_credential_retry<F, Fut, T>(
    instance_name: &str,
    mut operation: F,
) -> Result<T, SourceError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SourceError>>,
{
    match operation().await {
        Ok(value) => return Ok(value),
        Err(SourceError::InvalidCredentials { .. }) => {
            delete_source_credentials(instance_name).ok();
            eprintln!("[{instance_name}] credentials rejected, re-prompting and retrying once");
        }
        Err(other) => return Err(other),
    }

    operation().await
}

async fn execute_transfer_one(instance_name: &str, dry_run: bool) -> anyhow::Result<()> {
    let config = config::load()?;

    let source = config
        .source_instance(instance_name)
        .with_context(|| format!("Failed to find source instance \"{instance_name}\""))?;
    let kind = source.kind;

    let targets = build_sink_targets(instance_name, &config, dry_run)?;
    if targets.is_empty() {
        bail!("Source instance \"{instance_name}\" has no bindings configured");
    }

    let state = StateStore::open(&config::state_path()?).await?;
    let label = instance_label(instance_name, kind);

    let outcome = with_credential_retry(instance_name, || {
        transfer_attempt(instance_name, kind, &targets, &state, dry_run)
    })
    .await?;

    print_transfer_outcome(&label, &outcome, dry_run);
    Ok(())
}

async fn execute_transfer_many(filter: SourceFilter, dry_run: bool) -> anyhow::Result<()> {
    let config = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;

    let instances = filtered_instances(&config, &filter);

    if instances.is_empty() {
        bail!("Failed to find any configured sources");
    }

    let mut failed: Vec<String> = Vec::new();

    for (instance_name, kind) in instances {
        if shutdown::is_shutting_down() {
            break;
        }

        let label = instance_label(&instance_name, kind);

        let targets = match build_sink_targets(&instance_name, &config, dry_run) {
            Ok(targets) => targets,
            Err(error) => {
                log_other(&label, &error);
                failed.push(label);
                continue;
            }
        };

        if targets.is_empty() {
            eprintln!("[{label}] no bindings configured; skipping");
            continue;
        }

        let result = with_credential_retry(&instance_name, || {
            transfer_attempt(&instance_name, kind, &targets, &state, dry_run)
        })
        .await;

        match result {
            Ok(outcome) => print_transfer_outcome(&label, &outcome, dry_run),
            Err(error) => {
                log_source_error(&label, &error);
                failed.push(label);
            }
        }
    }

    if !failed.is_empty() {
        bail!("Failed to transfer: {}", failed.join(", "));
    }

    Ok(())
}

async fn execute_mark_one(
    instance_name: &str,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let config = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;

    let source = config
        .source_instance(instance_name)
        .with_context(|| format!("Failed to find source instance \"{instance_name}\""))?;
    let kind = source.kind;
    let sink_ids = sink_names_bound_to(instance_name, &config)?;

    if sink_ids.is_empty() {
        bail!("Source \"{instance_name}\" has no bindings to mark for");
    }

    let label = instance_label(instance_name, kind);

    let outcome = with_credential_retry(instance_name, || {
        mark_attempt(instance_name, kind, &sink_ids, &state, until, dry_run)
    })
    .await?;

    print_mark_outcome(&label, &outcome, dry_run);
    Ok(())
}

async fn execute_mark_many(
    filter: SourceFilter,
    until: Option<NaiveDate>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let config = config::load()?;
    let state = StateStore::open(&config::state_path()?).await?;

    let instances = filtered_instances(&config, &filter);

    if instances.is_empty() {
        bail!("Failed to find any configured sources");
    }

    let mut failed: Vec<String> = Vec::new();

    for (instance_name, kind) in instances {
        if shutdown::is_shutting_down() {
            break;
        }

        let label = instance_label(&instance_name, kind);

        let sink_ids = match sink_names_bound_to(&instance_name, &config) {
            Ok(ids) => ids,
            Err(error) => {
                log_other(&label, &error);
                failed.push(label);
                continue;
            }
        };

        if sink_ids.is_empty() {
            eprintln!("[{label}] no bindings to mark for; skipping");
            continue;
        }

        let result = with_credential_retry(&instance_name, || {
            mark_attempt(&instance_name, kind, &sink_ids, &state, until, dry_run)
        })
        .await;

        match result {
            Ok(outcome) => print_mark_outcome(&label, &outcome, dry_run),
            Err(error) => {
                log_source_error(&label, &error);
                failed.push(label);
            }
        }
    }

    if !failed.is_empty() {
        bail!("Failed to mark: {}", failed.join(", "));
    }

    Ok(())
}

fn filtered_instances(config: &Config, filter: &SourceFilter) -> Vec<(String, SourceKind)> {
    let mut instances: Vec<(String, SourceKind)> = config
        .sources
        .iter()
        .filter(|(_, instance)| match filter {
            SourceFilter::All => true,
            SourceFilter::Kind(kind) => instance.kind == *kind,
        })
        .map(|(name, instance)| (name.clone(), instance.kind))
        .collect();
    instances.sort_by(|a, b| a.0.cmp(&b.0));
    instances
}

fn build_sink_targets(
    source_name: &str,
    config: &Config,
    dry_run: bool,
) -> anyhow::Result<Vec<SinkTarget>> {
    let source = config
        .source_instance(source_name)
        .with_context(|| format!("Failed to find source instance \"{source_name}\""))?;

    let mut sink_names: Vec<&String> = source.sinks.keys().collect();
    sink_names.sort();

    let mut targets = Vec::with_capacity(sink_names.len());

    for sink_name in sink_names {
        let binding = source
            .sinks
            .get(sink_name)
            .expect("sink_name came from source.sinks");

        if dry_run {
            targets.push(SinkTarget::DryRun(sink_name.clone()));
            continue;
        }

        let sink_instance = config.sinks.get(sink_name).with_context(|| {
            format!(
                "Source \"{source_name}\" is bound to sink \"{sink_name}\" which is not configured"
            )
        })?;

        let sink: Box<dyn Sink> = match (sink_instance, binding) {
            (SinkInstance::Paperless { base_url }, SinkBinding::Paperless(binding)) => {
                let token = resolve_token(sink_name)?;
                let client = PaperlessClient::new(base_url.clone(), token)?;
                Box::new(PaperlessSink::new(sink_name, client, binding.clone()))
            }
            (SinkInstance::Filesystem { root }, SinkBinding::Filesystem(binding)) => Box::new(
                FilesystemSink::new(sink_name, root.clone(), binding.clone()),
            ),
            (sink_instance, _) => bail!(
                "binding for sink \"{sink_name}\" does not match its kind ({})",
                match sink_instance {
                    SinkInstance::Paperless { .. } => "paperless",
                    SinkInstance::Filesystem { .. } => "filesystem",
                }
            ),
        };

        targets.push(SinkTarget::Live(sink));
    }

    Ok(targets)
}

fn sink_names_bound_to(source_name: &str, config: &Config) -> anyhow::Result<Vec<String>> {
    let Some(source) = config.source_instance(source_name) else {
        return Ok(Vec::new());
    };

    let mut names: Vec<String> = source.sinks.keys().cloned().collect();
    names.sort();

    for name in &names {
        if !config.sinks.contains_key(name) {
            bail!("Source \"{source_name}\" is bound to sink \"{name}\" which is not configured");
        }
    }

    Ok(names)
}

fn log_source_error(label: &str, error: &SourceError) {
    match error {
        SourceError::InvalidCredentials { .. } => {
            eprintln!("[{label}] credentials still rejected after retry, keyring entry cleared");
        }
        SourceError::Other(inner) => {
            eprintln!("[{label}] failed: {inner:?}");
        }
    }
}

fn log_other(label: &str, error: &anyhow::Error) {
    eprintln!("[{label}] failed: {error:?}");
}
