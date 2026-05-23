use std::fmt;

use inquire::{Confirm, Select};
use strum::IntoEnumIterator;

use crate::config::{self, Config};
use crate::sources::SourceKind;
use crate::sources::wizard::SourceWizard;
use crate::state::StateStore;

mod bindings;

use super::prompts::{
    WizardAction, pause, prompt_new_name, render_header, sorted_keys, suggest_unique_name,
};

#[derive(Clone)]
enum ListEntry {
    Source { name: String, kind: SourceKind },
    AddNew,
    Back,
}

impl fmt::Display for ListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source { name, kind } => write!(f, "{name} ({})", kind.label()),
            Self::AddNew => write!(f, "+ Add new source"),
            Self::Back => write!(f, "Back"),
        }
    }
}

struct SourceKindPick(SourceKind);

impl fmt::Display for SourceKindPick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.label())
    }
}

#[derive(Clone)]
enum MenuChoice {
    Kind(WizardAction),
    ManageBindings,
    Rename,
    Delete,
    Back,
}

impl fmt::Display for MenuChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kind(action) => action.fmt(f),
            Self::ManageBindings => write!(f, "Manage bindings"),
            Self::Rename => write!(f, "Rename"),
            Self::Delete => write!(f, "Delete"),
            Self::Back => write!(f, "Back"),
        }
    }
}

pub(super) async fn run_menu(config: &mut Config, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Sources".to_string());

    loop {
        render_header(crumbs);

        let entries = build_list(config);
        let pick = Select::new("Choose:", entries).prompt()?;

        match pick {
            ListEntry::Source { name, kind } => {
                source_actions(config, &name, kind, crumbs).await?;
            }
            ListEntry::AddNew => add_source(config, crumbs).await?,
            ListEntry::Back => {
                crumbs.pop();
                return Ok(());
            }
        }
    }
}

fn build_list(config: &Config) -> Vec<ListEntry> {
    let mut entries: Vec<ListEntry> = sorted_keys(&config.sources)
        .into_iter()
        .map(|name| {
            let kind = config.sources[&name].kind;

            ListEntry::Source { name, kind }
        })
        .collect();

    entries.push(ListEntry::AddNew);
    entries.push(ListEntry::Back);
    entries
}

async fn add_source(config: &mut Config, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Add".to_string());
    render_header(crumbs);

    let kind = Select::new(
        "Source kind:",
        SourceKind::iter().map(SourceKindPick).collect(),
    )
    .prompt()?
    .0;

    let taken = sorted_keys(&config.sources);
    let default_name = suggest_unique_name(&taken, kind.name());
    let name = prompt_new_name(
        &format!("Instance name (default \"{default_name}\"):"),
        &default_name,
        taken,
        None,
    )?;

    let saved = kind.wizard().add(config, &name).await?;

    if saved {
        println!();
        println!("Source \"{name}\" added.");
        println!();
        bindings::offer_initial_binding(config, &name, crumbs).await?;
    }

    crumbs.pop();
    Ok(())
}

async fn source_actions(
    config: &mut Config,
    name: &str,
    kind: SourceKind,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push(name.to_string());
    let wizard = kind.wizard();

    loop {
        render_header(crumbs);
        print_source_summary(config, name, kind, wizard.as_ref());
        println!();

        let mut choices: Vec<MenuChoice> =
            wizard.actions().into_iter().map(MenuChoice::Kind).collect();
        choices.push(MenuChoice::ManageBindings);
        choices.push(MenuChoice::Rename);
        choices.push(MenuChoice::Delete);
        choices.push(MenuChoice::Back);

        match Select::new("Action:", choices).prompt()? {
            MenuChoice::Kind(action) => {
                wizard.run_action(action, config, name, crumbs).await?;
            }
            MenuChoice::ManageBindings => {
                bindings::manage_bindings(config, name, crumbs).await?;
            }
            MenuChoice::Rename => {
                if rename_source(config, name, wizard.as_ref(), crumbs).await? {
                    crumbs.pop();
                    return Ok(());
                }
            }
            MenuChoice::Delete => {
                if delete_source(config, name, wizard.as_ref(), crumbs).await? {
                    crumbs.pop();
                    return Ok(());
                }
            }
            MenuChoice::Back => {
                crumbs.pop();
                return Ok(());
            }
        }
    }
}

fn print_source_summary(config: &Config, name: &str, kind: SourceKind, wizard: &dyn SourceWizard) {
    println!("Source: {name} ({})", kind.label());
    wizard.print_summary(config, name, "  ");

    let Some(source) = config.source_instance(name) else {
        return;
    };

    if source.sinks.is_empty() {
        println!("  Bindings: (none)");
        return;
    }

    println!("  Bindings:");
    let mut sink_names: Vec<&String> = source.sinks.keys().collect();
    sink_names.sort();

    for sink_name in sink_names {
        let binding = &source.sinks[sink_name];
        let Some(sink) = config.sinks.get(sink_name) else {
            println!("    → {sink_name} (?)");
            continue;
        };

        let sink_kind = sink.kind();
        println!("    → {sink_name} ({})", sink_kind.label());
        sink_kind
            .wizard()
            .print_binding_details(binding, "        ");
    }
}

async fn rename_source(
    config: &mut Config,
    old_name: &str,
    wizard: &dyn SourceWizard,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    crumbs.push("Rename".to_string());
    render_header(crumbs);

    let taken = sorted_keys(&config.sources);
    let new_name = prompt_new_name("New name:", old_name, taken, Some(old_name.to_string()))?;

    if new_name == old_name {
        println!("(name unchanged)");
        pause()?;
        crumbs.pop();
        return Ok(false);
    }

    let state = StateStore::open(&config::state_path()?).await?;
    state.rename_source(old_name, &new_name).await?;
    wizard.on_rename(old_name, &new_name)?;
    config.rename_source_in_config(old_name, &new_name)?;
    config::save(config)?;

    println!();
    println!("Renamed source \"{old_name}\" to \"{new_name}\".");
    pause()?;
    crumbs.pop();
    Ok(true)
}

async fn delete_source(
    config: &mut Config,
    name: &str,
    wizard: &dyn SourceWizard,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    crumbs.push("Delete".to_string());
    render_header(crumbs);

    let binding_count = config
        .source_instance(name)
        .map(|source| source.sinks.len())
        .unwrap_or(0);

    let state = StateStore::open(&config::state_path()?).await?;
    let upload_count = state.count_uploads_for_source(name).await?;

    println!("Source \"{name}\" will be deleted.");
    println!("Affected:");
    println!("  - {binding_count} binding(s) from this source");
    println!("  - {upload_count} upload record(s)");
    println!();

    let confirmed = Confirm::new("Proceed?").with_default(false).prompt()?;

    if !confirmed {
        crumbs.pop();
        return Ok(false);
    }

    state.delete_uploads_for_source(name).await?;
    wizard.on_delete(name)?;
    config.remove_source(name);
    config::save(config)?;

    println!();
    println!("Source \"{name}\" deleted.");
    pause()?;
    crumbs.pop();
    Ok(true)
}
