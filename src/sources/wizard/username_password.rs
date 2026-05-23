use anyhow::Context;
use async_trait::async_trait;

use crate::config::{self, Config};
use crate::credentials::{delete_source_credentials, prompt_and_save_for_kind};
use crate::keystore::{self, source_account};
use crate::setup::prompts::{
    FailureChoice, WizardAction, pause, prompt_failure_choice, render_header,
};
use crate::sources::SourceKind;
use crate::sources::wizard::SourceWizard;

pub struct UsernamePasswordSourceWizard {
    kind: SourceKind,
}

impl UsernamePasswordSourceWizard {
    pub fn new(kind: SourceKind) -> Self {
        Self { kind }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumString, strum::IntoStaticStr,
)]
enum Action {
    ReenterCredentials,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::ReenterCredentials => "Re-enter credentials",
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
impl SourceWizard for UsernamePasswordSourceWizard {
    async fn add(&self, config: &mut Config, name: &str) -> anyhow::Result<bool> {
        prompt_and_save_for_kind(self.kind, name)?;

        loop {
            match self.kind.build(name).await {
                Ok(_) => {
                    config
                        .add_source_instance(name, self.kind)
                        .context("Failed to add source instance")?;
                    config::save(config)?;

                    return Ok(true);
                }
                Err(error) => match prompt_failure_choice(&anyhow::Error::from(error))? {
                    FailureChoice::Retry => {
                        prompt_and_save_for_kind(self.kind, name)?;
                    }
                    FailureChoice::SaveAnyway => {
                        config
                            .add_source_instance(name, self.kind)
                            .context("Failed to add source instance")?;
                        config::save(config)?;

                        println!();
                        println!("Source \"{name}\" saved (verification skipped).");
                        pause()?;

                        return Ok(true);
                    }
                    FailureChoice::Cancel => {
                        delete_source_credentials(name).ok();

                        println!();
                        println!("Cancelled. Nothing saved.");
                        pause()?;

                        return Ok(false);
                    }
                },
            }
        }
    }

    fn print_summary(&self, _config: &Config, name: &str, prefix: &str) {
        let creds_present = keystore::read_secret(&source_account(name))
            .ok()
            .flatten()
            .is_some();
        let creds_label = if creds_present { "set" } else { "missing" };

        println!("{prefix}Credentials: {creds_label}");
    }

    fn actions(&self) -> Vec<WizardAction> {
        use strum::IntoEnumIterator;

        Action::iter().map(Action::entry).collect()
    }

    async fn run_action(
        &self,
        action: WizardAction,
        _config: &mut Config,
        name: &str,
        crumbs: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let parsed: Action = action.id.parse().with_context(|| {
            format!(
                "UsernamePasswordSourceWizard: unknown action \"{}\"",
                action.id
            )
        })?;

        match parsed {
            Action::ReenterCredentials => reenter_credentials(self.kind, name, crumbs).await,
        }
    }

    fn on_delete(&self, name: &str) -> anyhow::Result<()> {
        delete_source_credentials(name).ok();

        Ok(())
    }

    fn on_rename(&self, old: &str, new: &str) -> anyhow::Result<()> {
        keystore::rename(&source_account(old), &source_account(new))?;

        Ok(())
    }
}

async fn reenter_credentials(
    kind: SourceKind,
    name: &str,
    crumbs: &mut Vec<String>,
) -> anyhow::Result<()> {
    crumbs.push("Re-enter credentials".to_string());
    render_header(crumbs);

    let previous = keystore::read_secret(&source_account(name))?;
    prompt_and_save_for_kind(kind, name)?;

    loop {
        match kind.build(name).await {
            Ok(_) => {
                println!();
                println!("Credentials accepted.");
                pause()?;
                crumbs.pop();

                return Ok(());
            }
            Err(error) => match prompt_failure_choice(&anyhow::Error::from(error))? {
                FailureChoice::Retry => {
                    prompt_and_save_for_kind(kind, name)?;
                }
                FailureChoice::SaveAnyway => {
                    println!();
                    println!("Credentials saved (verification skipped).");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
                FailureChoice::Cancel => {
                    if let Some(prev) = previous {
                        keystore::write_secret(&source_account(name), &prev)?;
                    } else {
                        delete_source_credentials(name).ok();
                    }

                    println!();
                    println!("Cancelled. Credentials restored.");
                    pause()?;
                    crumbs.pop();

                    return Ok(());
                }
            },
        }
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
