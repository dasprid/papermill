use async_trait::async_trait;

use crate::config::Config;
use crate::setup::prompts::WizardAction;

mod username_password;

pub use username_password::UsernamePasswordSourceWizard;

#[async_trait]
pub trait SourceWizard: Send + Sync {
    async fn add(&self, config: &mut Config, name: &str) -> anyhow::Result<bool>;

    fn print_summary(&self, config: &Config, name: &str, prefix: &str);

    /// Kind-specific actions for the per-source menu. Manage bindings,
    /// Rename, Delete, and Back are appended by setup.
    fn actions(&self) -> Vec<WizardAction>;

    async fn run_action(
        &self,
        action: WizardAction,
        config: &mut Config,
        name: &str,
        crumbs: &mut Vec<String>,
    ) -> anyhow::Result<()>;

    fn on_delete(&self, _name: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_rename(&self, _old: &str, _new: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
