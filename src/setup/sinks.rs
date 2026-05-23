use std::fmt;

use inquire::{Confirm, Select};
use strum::IntoEnumIterator;

use crate::config::{self, Config};
use crate::sinks::SinkKind;
use crate::sinks::wizard::SinkWizard;
use crate::state::StateStore;

use super::prompts::{
    WizardAction, pause, prompt_new_name, render_header, sorted_keys, suggest_unique_name,
};

#[derive(Clone)]
enum ListEntry {
    Sink { name: String, kind: SinkKind },
    AddNew,
    Back,
}

impl fmt::Display for ListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sink { name, kind } => write!(f, "{name} ({})", kind.label()),
            Self::AddNew => write!(f, "+ Add new sink"),
            Self::Back => write!(f, "Back"),
        }
    }
}

struct SinkKindPick(SinkKind);

impl fmt::Display for SinkKindPick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.label())
    }
}

pub(super) async fn run_menu(config: &mut Config, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Sinks".to_string());

    loop {
        render_header(crumbs);

        let entries = build_list(config);
        let pick = Select::new("Choose:", entries).prompt()?;

        match pick {
            ListEntry::Sink { name, kind } => sink_actions(config, &name, kind, crumbs).await?,
            ListEntry::AddNew => add_sink(config, crumbs).await?,
            ListEntry::Back => {
                crumbs.pop();
                return Ok(());
            }
        }
    }
}

fn build_list(config: &Config) -> Vec<ListEntry> {
    let mut entries: Vec<ListEntry> = sorted_keys(&config.sinks)
        .into_iter()
        .map(|name| {
            let kind = config.sinks[&name].kind();

            ListEntry::Sink { name, kind }
        })
        .collect();

    entries.push(ListEntry::AddNew);
    entries.push(ListEntry::Back);
    entries
}

async fn add_sink(config: &mut Config, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Add".to_string());
    render_header(crumbs);

    let kind = Select::new("Sink kind:", SinkKind::iter().map(SinkKindPick).collect())
        .prompt()?
        .0;

    let taken = sorted_keys(&config.sinks);
    let default_name = suggest_unique_name(&taken, kind.name());
    let name = prompt_new_name(
        &format!("Instance name (default \"{default_name}\"):"),
        &default_name,
        taken,
        None,
    )?;

    kind.wizard().add(config, &name).await?;

    crumbs.pop();
    Ok(())
}

#[derive(Clone)]
enum MenuChoice {
    Kind(WizardAction),
    Rename,
    Delete,
    Back,
}

impl fmt::Display for MenuChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kind(action) => action.fmt(f),
            Self::Rename => write!(f, "Rename"),
            Self::Delete => write!(f, "Delete"),
            Self::Back => write!(f, "Back"),
        }
    }
}

async fn sink_actions(
    config: &mut Config,
    name: &str,
    kind: SinkKind,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push(name.to_string());
    let wizard = kind.wizard();

    loop {
        render_header(crumbs);
        println!("Sink: {name} ({})", kind.label());
        wizard.print_summary(config, name, "  ");
        println!();

        let mut choices: Vec<MenuChoice> =
            wizard.actions().into_iter().map(MenuChoice::Kind).collect();
        choices.push(MenuChoice::Rename);
        choices.push(MenuChoice::Delete);
        choices.push(MenuChoice::Back);

        match Select::new("Action:", choices).prompt()? {
            MenuChoice::Kind(action) => {
                wizard.run_action(action, config, name, crumbs).await?;
            }
            MenuChoice::Rename => {
                if rename_sink(config, name, wizard.as_ref(), crumbs).await? {
                    crumbs.pop();
                    return Ok(());
                }
            }
            MenuChoice::Delete => {
                if delete_sink(config, name, wizard.as_ref(), crumbs).await? {
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

async fn rename_sink(
    config: &mut Config,
    old_name: &str,
    wizard: &dyn SinkWizard,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    crumbs.push("Rename".to_string());
    render_header(crumbs);

    let taken = sorted_keys(&config.sinks);
    let new_name = prompt_new_name("New name:", old_name, taken, Some(old_name.to_string()))?;

    if new_name == old_name {
        println!("(name unchanged)");
        pause()?;
        crumbs.pop();
        return Ok(false);
    }

    let state = StateStore::open(&config::state_path()?).await?;
    state.rename_sink(old_name, &new_name).await?;
    wizard.on_rename(old_name, &new_name)?;
    config.rename_sink_in_config(old_name, &new_name)?;
    config::save(config)?;

    println!();
    println!("Renamed sink \"{old_name}\" to \"{new_name}\".");
    pause()?;
    crumbs.pop();
    Ok(true)
}

async fn delete_sink(
    config: &mut Config,
    name: &str,
    wizard: &dyn SinkWizard,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    crumbs.push("Delete".to_string());
    render_header(crumbs);

    let binding_count = config
        .sources
        .values()
        .filter(|source| source.sinks.contains_key(name))
        .count();

    let state = StateStore::open(&config::state_path()?).await?;
    let upload_count = state.count_uploads_for_sink(name).await?;

    println!("Sink \"{name}\" will be deleted.");
    println!("Affected:");
    println!("  - {binding_count} source binding(s)");
    println!("  - {upload_count} upload record(s)");
    println!();

    let confirmed = Confirm::new("Proceed?").with_default(false).prompt()?;

    if !confirmed {
        crumbs.pop();
        return Ok(false);
    }

    state.delete_uploads_for_sink(name).await?;
    wizard.on_delete(name)?;
    config.delete_sink(name);
    config::save(config)?;

    println!();
    println!("Sink \"{name}\" deleted.");
    pause()?;
    crumbs.pop();
    Ok(true)
}
