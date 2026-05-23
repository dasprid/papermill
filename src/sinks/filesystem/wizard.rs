use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use inquire::validator::Validation;
use inquire::{Confirm, Text};

use crate::config::{self, Config, SinkBinding, SinkInstance};
use crate::setup::prompts::{
    FailureChoice, WizardAction, pause, prompt_failure_choice, render_header,
};
use crate::sinks::filesystem::{
    DEFAULT_FILENAME_TEMPLATE, FilesystemBinding, validate_filename_template, verify,
};
use crate::sinks::wizard::SinkWizard;

pub struct FilesystemSinkWizard;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumString, strum::IntoStaticStr,
)]
enum Action {
    EditRoot,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::EditRoot => "Edit root path",
        }
    }

    fn entry(self) -> WizardAction {
        WizardAction {
            id: self.into(),
            label: self.label(),
        }
    }
}

#[async_trait]
impl SinkWizard for FilesystemSinkWizard {
    async fn add(&self, config: &mut Config, name: &str) -> anyhow::Result<bool> {
        let mut root = prompt_filesystem_root(None)?;

        loop {
            match verify(&root) {
                Ok(()) => {
                    config.set_filesystem_sink(name, root)?;
                    config::save(config)?;
                    println!();
                    println!("Sink \"{name}\" added and verified.");
                    pause()?;

                    return Ok(true);
                }
                Err(error) => match prompt_failure_choice(&error)? {
                    FailureChoice::Retry => {
                        root = prompt_filesystem_root(Some(&root))?;
                    }
                    FailureChoice::SaveAnyway => {
                        config.set_filesystem_sink(name, root)?;
                        config::save(config)?;
                        println!();
                        println!("Sink \"{name}\" saved (verification skipped).");
                        pause()?;

                        return Ok(true);
                    }
                    FailureChoice::Cancel => {
                        println!();
                        println!("Cancelled. Nothing saved.");
                        pause()?;

                        return Ok(false);
                    }
                },
            }
        }
    }

    fn print_summary(&self, config: &Config, name: &str, prefix: &str) {
        if let Some(SinkInstance::Filesystem { root }) = config.sinks.get(name) {
            println!("{prefix}Root: {}", root.display());
        }
    }

    fn print_binding_details(&self, binding: &SinkBinding, prefix: &str) {
        let SinkBinding::Filesystem(binding) = binding else {
            return;
        };

        let subdir = binding.subdir.as_deref().unwrap_or("(none)");
        let group = if binding.group_by_year { "yes" } else { "no" };

        println!("{prefix}Subdirectory:      {subdir}");
        println!("{prefix}Group by year:     {group}");
        println!("{prefix}Filename template: {}", binding.filename_template);
    }

    fn actions(&self) -> Vec<WizardAction> {
        use strum::IntoEnumIterator;

        Action::iter().map(Action::entry).collect()
    }

    async fn run_action(
        &self,
        action: WizardAction,
        config: &mut Config,
        name: &str,
        crumbs: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let parsed: Action = action
            .id
            .parse()
            .with_context(|| format!("FilesystemSinkWizard: unknown action \"{}\"", action.id))?;

        match parsed {
            Action::EditRoot => edit_root(config, name, crumbs),
        }
    }

    fn default_binding(&self) -> SinkBinding {
        SinkBinding::Filesystem(FilesystemBinding {
            subdir: None,
            group_by_year: false,
            filename_template: DEFAULT_FILENAME_TEMPLATE.to_string(),
        })
    }

    async fn prompt_binding(
        &self,
        _config: &Config,
        _sink_name: &str,
        current: SinkBinding,
    ) -> anyhow::Result<SinkBinding> {
        let SinkBinding::Filesystem(current) = current else {
            anyhow::bail!("binding kind mismatch for filesystem sink");
        };

        let subdir_raw = Text::new("Subdirectory under the sink root (optional, blank for none):")
            .with_default(current.subdir.as_deref().unwrap_or(""))
            .prompt()?;
        let subdir = if subdir_raw.trim().is_empty() {
            None
        } else {
            Some(subdir_raw.trim().to_string())
        };

        let group_by_year = Confirm::new("Group invoices into per-year subdirectories?")
            .with_default(current.group_by_year)
            .prompt()?;

        let template_validator =
            |value: &str| -> Result<Validation, Box<dyn std::error::Error + Send + Sync>> {
                match validate_filename_template(value) {
                    Ok(()) => Ok(Validation::Valid),
                    Err(error) => Ok(Validation::Invalid(error.to_string().into())),
                }
            };

        let filename_template = Text::new("Filename template (tokens: {date}, {number}, {source}):")
            .with_default(&current.filename_template)
            .with_help_message(
                "{number} is required for uniqueness; include {source} if multiple sources share a sink root",
            )
            .with_validator(template_validator)
            .prompt()?;

        Ok(SinkBinding::Filesystem(FilesystemBinding {
            subdir,
            group_by_year,
            filename_template,
        }))
    }
}

fn edit_root(config: &mut Config, name: &str, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Edit root".to_string());
    render_header(crumbs);

    let current_root = match config.sinks.get(name) {
        Some(SinkInstance::Filesystem { root }) => root.clone(),
        _ => {
            println!("(not a filesystem sink)");
            pause()?;
            crumbs.pop();

            return Ok(());
        }
    };

    let mut root = prompt_filesystem_root(Some(&current_root))?;

    loop {
        match verify(&root) {
            Ok(()) => {
                config.set_filesystem_sink(name, root)?;
                config::save(config)?;
                println!();
                println!("Root path updated and verified.");
                pause()?;
                crumbs.pop();

                return Ok(());
            }
            Err(error) => match prompt_failure_choice(&error)? {
                FailureChoice::Retry => {
                    root = prompt_filesystem_root(Some(&root))?;
                }
                FailureChoice::SaveAnyway => {
                    config.set_filesystem_sink(name, root)?;
                    config::save(config)?;
                    println!();
                    println!("Root path saved (verification skipped).");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
                FailureChoice::Cancel => {
                    println!();
                    println!("Cancelled. Root path unchanged.");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
            },
        }
    }
}

fn prompt_filesystem_root(current: Option<&Path>) -> anyhow::Result<PathBuf> {
    let existing = current.map(|p| p.display().to_string());

    loop {
        let mut prompt = Text::new("Filesystem root directory:");

        if let Some(ref value) = existing {
            prompt = prompt.with_default(value);
        }

        let value = prompt.prompt()?;
        let trimmed = value.trim();

        if trimmed.is_empty() {
            eprintln!("Root cannot be empty.");
            continue;
        }

        return Ok(PathBuf::from(trimmed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn action_ids_round_trip() {
        for action in Action::iter() {
            let id: &'static str = action.into();
            let parsed: Action = id.parse().expect("variant id must parse back");
            assert_eq!(parsed, action);
        }
    }
}
