pub mod claude;
pub mod codex;
pub mod gemini;

use crate::core::state::ForgeState;
use crate::core::task::Task;
use std::path::Path;
use std::process::Command;

/// Trait for tool-specific adapters that render .forge/ state into native config
pub trait ToolAdapter {
    /// Name of the adapter (e.g., "claude", "codex", "gemini")
    fn name(&self) -> &str;

    /// Render the current state into the tool's native config file(s)
    fn render_config(
        &self,
        state: &ForgeState,
        tasks: &[Task],
        project_root: &Path,
    ) -> anyhow::Result<()>;

    /// Build the CLI command for headless execution (adapter-specific).
    fn build_command(
        &self,
        task: &Task,
        project_root: &Path,
        auth_mode: &str,
        permissions: &str,
    ) -> Command;

    /// Execute a task headlessly using this tool (sync, blocking).
    /// Default implementation: calls `build_command()` + `.output()` + `process_output()`.
    fn execute_headless(
        &self,
        task: &Task,
        project_root: &Path,
        auth_mode: &str,
        permissions: &str,
    ) -> anyhow::Result<ExecutionResult> {
        let mut cmd = self.build_command(task, project_root, auth_mode, permissions);
        let output = cmd.output()?;
        Ok(process_output(output))
    }
}

/// Execute a pre-built `std::process::Command` asynchronously via tokio.
pub async fn execute_command_async(cmd: Command) -> anyhow::Result<ExecutionResult> {
    let mut tokio_cmd = tokio::process::Command::from(cmd);
    let output = tokio_cmd.output().await?;
    Ok(process_output(output))
}

/// Convert raw process output into an `ExecutionResult`.
pub fn process_output(output: std::process::Output) -> ExecutionResult {
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n\nSTDERR:\n{stderr}")
    };

    ExecutionResult {
        success: output.status.success(),
        output: combined,
        exit_code,
    }
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: String,
    pub exit_code: i32,
}
