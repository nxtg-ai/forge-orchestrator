pub mod claude;
pub mod codex;
pub mod gemini;

use crate::core::state::ForgeState;
use crate::core::task::Task;
use std::path::Path;

/// Trait for tool-specific adapters that render .forge/ state into native config
pub trait ToolAdapter {
    /// Name of the adapter (e.g., "claude", "codex", "gemini")
    fn name(&self) -> &str;

    /// Render the current state into the tool's native config file(s)
    fn render_config(&self, state: &ForgeState, tasks: &[Task], project_root: &Path)
        -> anyhow::Result<()>;

    /// Execute a task headlessly using this tool
    fn execute_headless(
        &self,
        task: &Task,
        project_root: &Path,
    ) -> anyhow::Result<ExecutionResult>;
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: String,
    pub exit_code: i32,
}
