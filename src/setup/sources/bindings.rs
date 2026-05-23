use std::fmt;

use inquire::{Confirm, Select};

use crate::config::{self, Config, SinkInstance};
use crate::sinks::SinkKind;

use crate::setup::prompts::{pause, pick_with_cancel, render_header};

#[derive(Clone)]
enum BindingListEntry {
    Binding {
        sink_name: String,
        sink_kind: SinkKind,
    },
    Orphan {
        sink_name: String,
    },
    AddNew,
    Back,
}

impl fmt::Display for BindingListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding {
                sink_name,
                sink_kind,
            } => write!(f, "→ {sink_name} ({})", sink_kind.label()),
            Self::Orphan { sink_name } => {
                write!(f, "→ {sink_name} (broken: sink no longer exists)")
            }
            Self::AddNew => write!(f, "+ Add binding"),
            Self::Back => write!(f, "Back"),
        }
    }
}

#[derive(Clone)]
enum BindingAction {
    Edit,
    Remove,
    Back,
}

impl fmt::Display for BindingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit => write!(f, "Edit"),
            Self::Remove => write!(f, "Remove"),
            Self::Back => write!(f, "Back"),
        }
    }
}

pub(super) async fn offer_initial_binding(
    config: &mut Config,
    source_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    loop {
        let candidates = available_sinks_to_bind(config, source_name);

        if candidates.is_empty() {
            println!("(no sinks available to bind; add a sink first)");
            pause()?;
            return Ok(());
        }

        let proceed = Confirm::new("Bind this source to a sink now?")
            .with_default(true)
            .prompt()?;

        if !proceed {
            return Ok(());
        }

        add_binding_inner(config, source_name, &candidates, crumbs).await?;

        let more = Confirm::new("Add another binding?")
            .with_default(false)
            .prompt()?;

        if !more {
            return Ok(());
        }
    }
}

pub(super) async fn manage_bindings(
    config: &mut Config,
    source_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push("Bindings".to_string());

    loop {
        render_header(crumbs);

        let entries = build_binding_list(config, source_name);
        let pick = Select::new("Choose:", entries).prompt()?;

        match pick {
            BindingListEntry::Binding { sink_name, .. } => {
                binding_actions(config, source_name, &sink_name, crumbs).await?;
            }
            BindingListEntry::Orphan { sink_name } => {
                orphan_binding_action(config, source_name, &sink_name, crumbs)?;
            }
            BindingListEntry::AddNew => {
                let candidates = available_sinks_to_bind(config, source_name);

                if candidates.is_empty() {
                    println!();
                    println!(
                        "(no available sinks to bind; add a sink first, or all sinks are already bound)"
                    );
                    pause()?;
                    continue;
                }

                add_binding_inner(config, source_name, &candidates, crumbs).await?;
            }
            BindingListEntry::Back => {
                crumbs.pop();
                return Ok(());
            }
        }
    }
}

fn build_binding_list(config: &Config, source_name: &str) -> Vec<BindingListEntry> {
    let mut entries: Vec<BindingListEntry> = config
        .source_instance(source_name)
        .map(|source| {
            let mut names: Vec<&String> = source.sinks.keys().collect();
            names.sort();
            names
                .into_iter()
                .map(|sink_name| match config.sinks.get(sink_name) {
                    Some(sink) => BindingListEntry::Binding {
                        sink_name: sink_name.clone(),
                        sink_kind: sink.kind(),
                    },
                    None => BindingListEntry::Orphan {
                        sink_name: sink_name.clone(),
                    },
                })
                .collect()
        })
        .unwrap_or_default();

    entries.push(BindingListEntry::AddNew);
    entries.push(BindingListEntry::Back);
    entries
}

fn available_sinks_to_bind(config: &Config, source_name: &str) -> Vec<String> {
    let bound: Vec<String> = config
        .source_instance(source_name)
        .map(|s| s.sinks.keys().cloned().collect())
        .unwrap_or_default();

    let mut names: Vec<String> = config
        .sinks
        .keys()
        .filter(|name| !bound.contains(name))
        .cloned()
        .collect();
    names.sort();
    names
}

async fn add_binding_inner(
    config: &mut Config,
    source_name: &str,
    candidates: &[String],
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push("Add binding".to_string());
    render_header(crumbs);

    let Some(sink_name) = pick_with_cancel("Bind to which sink?", candidates)? else {
        crumbs.pop();
        return Ok(());
    };

    let Some(sink_kind) = config.sinks.get(&sink_name).map(SinkInstance::kind) else {
        crumbs.pop();
        return Ok(());
    };

    let sink_wizard = sink_kind.wizard();
    let initial = sink_wizard.default_binding();

    let binding = match sink_wizard
        .prompt_binding(config, &sink_name, initial)
        .await
    {
        Ok(binding) => binding,
        Err(error) => {
            print_binding_setup_failure(&sink_name, &error);
            pause()?;
            crumbs.pop();
            return Ok(());
        }
    };

    config.set_binding(source_name, &sink_name, binding)?;
    config::save(config)?;

    println!();
    println!("Bound \"{source_name}\" → \"{sink_name}\".");
    pause()?;
    crumbs.pop();
    Ok(())
}

async fn binding_actions(
    config: &mut Config,
    source_name: &str,
    sink_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push(format!("→ {sink_name}"));

    loop {
        render_header(crumbs);

        let Some(binding) = config
            .source_instance(source_name)
            .and_then(|source| source.sinks.get(sink_name))
        else {
            crumbs.pop();
            return Ok(());
        };

        let Some(sink_kind) = config.sinks.get(sink_name).map(SinkInstance::kind) else {
            crumbs.pop();
            return Ok(());
        };

        let sink_wizard = sink_kind.wizard();

        println!(
            "Binding: {source_name} → {sink_name} ({})",
            sink_kind.label()
        );
        sink_wizard.print_binding_details(binding, "  ");
        println!();

        let action = Select::new(
            "Action:",
            vec![
                BindingAction::Edit,
                BindingAction::Remove,
                BindingAction::Back,
            ],
        )
        .prompt()?;

        match action {
            BindingAction::Edit => edit_binding(config, source_name, sink_name, crumbs).await?,
            BindingAction::Remove => {
                if remove_binding(config, source_name, sink_name, crumbs)? {
                    crumbs.pop();
                    return Ok(());
                }
            }
            BindingAction::Back => {
                crumbs.pop();
                return Ok(());
            }
        }
    }
}

async fn edit_binding(
    config: &mut Config,
    source_name: &str,
    sink_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push("Edit".to_string());
    render_header(crumbs);

    let Some(sink_kind) = config.sinks.get(sink_name).map(SinkInstance::kind) else {
        crumbs.pop();
        return Ok(());
    };

    let sink_wizard = sink_kind.wizard();

    let current = match config
        .source_instance(source_name)
        .and_then(|s| s.sinks.get(sink_name))
        .cloned()
    {
        Some(binding) => binding,
        None => sink_wizard.default_binding(),
    };

    let binding = match sink_wizard.prompt_binding(config, sink_name, current).await {
        Ok(binding) => binding,
        Err(error) => {
            print_binding_setup_failure(sink_name, &error);
            pause()?;
            crumbs.pop();
            return Ok(());
        }
    };

    config.set_binding(source_name, sink_name, binding)?;
    config::save(config)?;

    println!();
    println!("Updated binding \"{source_name}\" → \"{sink_name}\".");
    pause()?;
    crumbs.pop();
    Ok(())
}

fn orphan_binding_action(
    config: &mut Config,
    source_name: &str,
    sink_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push(format!("→ {sink_name} (broken)"));
    render_header(crumbs);

    println!("Binding refers to sink \"{sink_name}\" which is not configured.");
    println!();

    let confirmed = Confirm::new(&format!(
        "Remove binding \"{source_name}\" → \"{sink_name}\"?",
    ))
    .with_default(true)
    .prompt()?;

    if confirmed {
        config.remove_binding(source_name, sink_name);
        config::save(config)?;
        println!();
        println!("Removed binding \"{source_name}\" → \"{sink_name}\".");
    }

    pause()?;
    crumbs.pop();
    Ok(())
}

fn remove_binding(
    config: &mut Config,
    source_name: &str,
    sink_name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    crumbs.push("Remove".to_string());
    render_header(crumbs);

    let confirmed = Confirm::new(&format!(
        "Really remove binding \"{source_name}\" → \"{sink_name}\"?",
    ))
    .with_default(false)
    .prompt()?;

    if !confirmed {
        crumbs.pop();
        return Ok(false);
    }

    config.remove_binding(source_name, sink_name);
    config::save(config)?;

    println!();
    println!("Removed binding \"{source_name}\" → \"{sink_name}\".");
    pause()?;
    crumbs.pop();
    Ok(true)
}

fn print_binding_setup_failure(sink_name: &str, error: &anyhow::Error) {
    println!();
    eprintln!("Could not configure binding to sink \"{sink_name}\":");
    eprintln!();
    eprintln!("    {error:?}");
    eprintln!();
    eprintln!("Check the sink under Sinks, then try the binding again.");
    println!();
}
