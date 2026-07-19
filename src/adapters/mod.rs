pub mod claude;
pub mod codex;
pub mod gemini;

use crate::core::state::ForgeState;
use crate::core::task::Task;
use std::path::Path;
use std::process::Command;

/// Read the digit run immediately AFTER the last occurrence of `prefix`, as a clamped percent.
/// Case-insensitive on the caller's side (pass a lowercased haystack). Pure.
pub(crate) fn pct_after(haystack: &str, prefix: &str) -> Option<u8> {
    let idx = haystack.rfind(prefix)?;
    let rest = &haystack[idx + prefix.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok().map(|n| n.min(100) as u8)
}

/// Read the digit run immediately BEFORE the last occurrence of `suffix`, as a clamped percent.
pub(crate) fn pct_before(haystack: &str, suffix: &str) -> Option<u8> {
    let idx = haystack.rfind(suffix)?;
    let head = &haystack[..idx];
    let mut digits: Vec<char> = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.reverse();
    let digits: String = digits.into_iter().collect();
    digits.parse::<u32>().ok().map(|n| n.min(100) as u8)
}

/// Trait for tool-specific adapters that render .forge/ state into native config
pub trait ToolAdapter {
    /// Name of the adapter (e.g., "claude", "codex", "gemini")
    fn name(&self) -> &str;

    /// Extract the context-**used** percent (0..=100) from this tool's statusline gauge, or `None`
    /// when the tool exposes no such gauge in the given text (W2-C fleet HUD).
    ///
    /// This is deliberately per-adapter, not one shared regex: each tool speaks a different dialect
    /// — Claude renders `ctx:NN%` (used), Codex renders `NN% left` (remaining, so `100 - NN`),
    /// Gemini/Antigravity expose none (→ `None`, surfaced as "n/a", never guessed). The gauge is the
    /// ONLY thing read from a pane; the rest of the captured text is discarded by the caller. Pure.
    fn parse_ctx_pct(&self, _statusline: &str) -> Option<u8> {
        None
    }

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

    /// Build the CLI command for interactive PTY execution (Stargate mode).
    /// This should launch the CLI's native interactive TUI (no exec/pipe flags).
    /// Default: delegates to build_command(). Override to launch actual TUIs.
    fn build_command_interactive(
        &self,
        task: &Task,
        project_root: &Path,
        auth_mode: &str,
        permissions: &str,
    ) -> Command {
        self.build_command(task, project_root, auth_mode, permissions)
    }

    /// Text to type into the PTY after spawning the interactive TUI.
    /// Written after a short delay to let the TUI initialize.
    /// Default: None (no initial input).
    fn initial_input(&self, _task: &Task) -> Option<String> {
        None
    }

    /// Delay in milliseconds before typing initial_input into the PTY (fallback timeout).
    /// Only used if ready_pattern() returns None, or as max wait for pattern detection.
    /// Default: 10000ms.
    fn initial_input_delay_ms(&self) -> u64 {
        10000
    }

    /// Pattern to detect in PTY output indicating the TUI is ready for input.
    /// When this text appears in the terminal output, initial_input is typed immediately.
    /// Falls back to initial_input_delay_ms() timeout if pattern never appears.
    /// Default: None (use fixed delay).
    fn ready_pattern(&self) -> Option<&str> {
        None
    }

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
