use anyhow::Context;
use async_trait::async_trait;
use inquire::{Password, PasswordDisplayMode, Text};
use reqwest::Url;
use secrecy::SecretString;

use crate::config::{self, Config, PaperlessBinding, SinkBinding, SinkInstance};
use crate::keystore;
use crate::setup::prompts::{
    FailureChoice, Pickable, WizardAction, pause, pick_multiple, pick_optional,
    prompt_failure_choice, render_header,
};
use crate::sinks::paperless::{
    NamedItem, PaperlessClient, delete_token, read_token, verify, write_token,
};
use crate::sinks::wizard::SinkWizard;

pub struct PaperlessSinkWizard;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumString, strum::IntoStaticStr,
)]
enum Action {
    EditUrl,
    ReplaceToken,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::EditUrl => "Edit URL",
            Self::ReplaceToken => "Replace API token",
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
impl SinkWizard for PaperlessSinkWizard {
    async fn add(&self, config: &mut Config, name: &str) -> anyhow::Result<bool> {
        let mut base_url = prompt_paperless_base_url(None)?;
        let mut token = prompt_paperless_token()?;

        loop {
            match verify(base_url.clone(), token.clone()).await {
                Ok(()) => {
                    write_token(name, &token)?;
                    config.set_paperless_sink(name, base_url)?;
                    config::save(config)?;
                    println!();
                    println!("Sink \"{name}\" added and verified.");
                    pause()?;

                    return Ok(true);
                }
                Err(error) => match prompt_failure_choice(&error)? {
                    FailureChoice::Retry => {
                        base_url = prompt_paperless_base_url(Some(&base_url))?;
                        token = prompt_paperless_token()?;
                    }
                    FailureChoice::SaveAnyway => {
                        write_token(name, &token)?;
                        config.set_paperless_sink(name, base_url)?;
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
        let token_present = keystore::read_secret(&keystore::sink_account(name))
            .ok()
            .flatten()
            .is_some();
        let token_label = if token_present { "set" } else { "missing" };

        if let Some(SinkInstance::Paperless { base_url }) = config.sinks.get(name) {
            println!("{prefix}URL:   {base_url}");
        }

        println!("{prefix}Token: {token_label}");
    }

    fn print_binding_details(&self, binding: &SinkBinding, prefix: &str) {
        let SinkBinding::Paperless(binding) = binding else {
            return;
        };

        let correspondent = binding
            .correspondent_id
            .map(|id| format!("#{id}"))
            .unwrap_or_else(|| "(none)".to_string());
        let document_type = binding
            .document_type_id
            .map(|id| format!("#{id}"))
            .unwrap_or_else(|| "(none)".to_string());
        let tags = if binding.tag_ids.is_empty() {
            "(none)".to_string()
        } else {
            binding
                .tag_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!("{prefix}Correspondent: {correspondent}");
        println!("{prefix}Document type: {document_type}");
        println!("{prefix}Tags:          {tags}");
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
            .with_context(|| format!("PaperlessSinkWizard: unknown action \"{}\"", action.id))?;

        match parsed {
            Action::EditUrl => edit_url(config, name, crumbs).await,
            Action::ReplaceToken => replace_token(config, name, crumbs).await,
        }
    }

    fn on_delete(&self, name: &str) -> anyhow::Result<()> {
        delete_token(name).ok();

        Ok(())
    }

    fn on_rename(&self, old: &str, new: &str) -> anyhow::Result<()> {
        keystore::rename(&keystore::sink_account(old), &keystore::sink_account(new))?;

        Ok(())
    }

    fn default_binding(&self) -> SinkBinding {
        SinkBinding::Paperless(PaperlessBinding::default())
    }

    async fn prompt_binding(
        &self,
        config: &Config,
        sink_name: &str,
        current: SinkBinding,
    ) -> anyhow::Result<SinkBinding> {
        let SinkBinding::Paperless(current) = current else {
            anyhow::bail!("binding kind mismatch for sink \"{sink_name}\"");
        };

        let base_url = match config.sinks.get(sink_name) {
            Some(SinkInstance::Paperless { base_url }) => base_url.clone(),
            _ => anyhow::bail!("sink \"{sink_name}\" is not a paperless sink"),
        };

        let token = read_token(sink_name)?.with_context(|| {
            format!("No token saved for sink \"{sink_name}\" (configure it first)")
        })?;

        let client = PaperlessClient::new(base_url, token)?;
        let correspondents = client.list_correspondents().await?;
        let document_types = client.list_document_types().await?;
        let tags = client.list_tags().await?;

        let correspondent_id = pick_optional(
            "Correspondent (optional):",
            &correspondents,
            current.correspondent_id,
        )?;

        let document_type_id = pick_optional(
            "Document type (optional):",
            &document_types,
            current.document_type_id,
        )?;

        let tag_ids = pick_multiple("Tags (optional, space to toggle):", &tags, &current.tag_ids)?;

        Ok(SinkBinding::Paperless(PaperlessBinding {
            correspondent_id,
            document_type_id,
            tag_ids,
        }))
    }
}

impl Pickable for NamedItem {
    fn id(&self) -> u32 {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }
}

async fn edit_url(config: &mut Config, name: &str, crumbs: &mut Vec<String>) -> anyhow::Result<()> {
    crumbs.push("Edit URL".to_string());
    render_header(crumbs);

    let current_url = match config.sinks.get(name) {
        Some(SinkInstance::Paperless { base_url }) => base_url.clone(),
        _ => {
            println!("(not a paperless sink)");
            pause()?;
            crumbs.pop();

            return Ok(());
        }
    };

    let token = read_token(name)?
        .with_context(|| format!("No token saved for sink \"{name}\"; set one first"))?;

    let mut base_url = prompt_paperless_base_url(Some(&current_url))?;

    loop {
        match verify(base_url.clone(), token.clone()).await {
            Ok(()) => {
                config.set_paperless_sink(name, base_url)?;
                config::save(config)?;
                println!();
                println!("URL updated and verified.");
                pause()?;
                crumbs.pop();

                return Ok(());
            }
            Err(error) => match prompt_failure_choice(&error)? {
                FailureChoice::Retry => {
                    base_url = prompt_paperless_base_url(Some(&base_url))?;
                }
                FailureChoice::SaveAnyway => {
                    config.set_paperless_sink(name, base_url)?;
                    config::save(config)?;
                    println!();
                    println!("URL saved (verification skipped).");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
                FailureChoice::Cancel => {
                    println!();
                    println!("Cancelled. URL unchanged.");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
            },
        }
    }
}

async fn replace_token(
    config: &Config,
    name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push("Replace token".to_string());
    render_header(crumbs);

    let base_url = match config.sinks.get(name) {
        Some(SinkInstance::Paperless { base_url }) => base_url.clone(),
        _ => {
            println!("(not a paperless sink)");
            pause()?;
            crumbs.pop();

            return Ok(());
        }
    };

    let previous_token = keystore::read_secret(&keystore::sink_account(name))?;
    let mut token = prompt_paperless_token()?;

    loop {
        match verify(base_url.clone(), token.clone()).await {
            Ok(()) => {
                write_token(name, &token)?;
                println!();
                println!("Token replaced and verified.");
                pause()?;
                crumbs.pop();

                return Ok(());
            }
            Err(error) => match prompt_failure_choice(&error)? {
                FailureChoice::Retry => {
                    token = prompt_paperless_token()?;
                }
                FailureChoice::SaveAnyway => {
                    write_token(name, &token)?;
                    println!();
                    println!("Token saved (verification skipped).");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
                FailureChoice::Cancel => {
                    if let Some(prev) = previous_token {
                        write_token(name, &prev)?;
                    }

                    println!();
                    println!("Cancelled. Token unchanged.");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
            },
        }
    }
}

fn prompt_paperless_base_url(current: Option<&Url>) -> anyhow::Result<Url> {
    let existing = current.map(Url::to_string);

    loop {
        let mut prompt = Text::new("Paperless base URL:");

        if let Some(ref value) = existing {
            prompt = prompt.with_default(value);
        }

        let value = prompt.prompt()?;
        let trimmed = value.trim();

        if trimmed.is_empty() {
            eprintln!("URL cannot be empty.");
            continue;
        }

        match Url::parse(trimmed) {
            Ok(url) => return Ok(config::ensure_trailing_slash(url)),
            Err(error) => {
                eprintln!("Invalid URL: {error}. Try again.");
            }
        }
    }
}

fn prompt_paperless_token() -> anyhow::Result<SecretString> {
    let raw = Password::new("Paperless API token:")
        .with_display_mode(PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .context("Failed to read paperless token")?;

    if raw.is_empty() {
        anyhow::bail!("Paperless token cannot be empty");
    }

    Ok(SecretString::from(raw))
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
