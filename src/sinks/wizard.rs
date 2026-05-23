use async_trait::async_trait;

use crate::config::{Config, SinkBinding};
use crate::setup::prompts::WizardAction;

#[async_trait]
pub trait SinkWizard: Send + Sync {
    async fn add(&self, config: &mut Config, name: &str) -> anyhow::Result<bool>;

    fn print_summary(&self, config: &Config, name: &str, prefix: &str);

    fn print_binding_details(&self, binding: &SinkBinding, prefix: &str);

    /// Kind-specific actions the wizard offers in the per-sink action menu.
    /// Rename/Delete/Back are appended by setup.
    fn actions(&self) -> Vec<WizardAction>;

    /// Run a kind-specific action picked from `actions()`. Setup never invokes
    /// this with an action that didn't come from `actions()`.
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

    fn default_binding(&self) -> SinkBinding;

    async fn prompt_binding(
        &self,
        config: &Config,
        sink_name: &str,
        current: SinkBinding,
    ) -> anyhow::Result<SinkBinding>;
}
