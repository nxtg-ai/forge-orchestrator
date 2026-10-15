use super::ToolAdapter;
use crate::core::state::ForgeState;
use crate::core::task::{AgentType, Task};
use std::path::Path;
use std::process::Command;

pub struct CodexAdapter;

/// The sandbox Codex runs a Forge task in when it runs unattended (`yolo`, and every PTY pane).
const UNATTENDED_SANDBOX: &str = "workspace-write";

/// Quote `s` as a TOML basic string, for a `codex -c key=value` override (codex parses the value as
/// TOML). Escapes backslash, double quote and control characters, so a Windows path stays one value.
fn toml_basic_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl CodexAdapter {
    /// The `codex exec` flags for an unattended Forge task (DIRECTIVE-NXTG-20261015-07).
    ///
    /// codex-cli 0.162.1 rejects `--full-auto` ("unexpected argument"), which every earlier Forge
    /// release passed. Its replacement here is two parts:
    /// - `--sandbox workspace-write`: the model's shell commands may write only inside the project,
    ///   the sandbox half of what `--full-auto` meant. `codex exec` never waits on a person (its
    ///   approval policy is `never`), so no flag is needed for that.
    /// - the `forge` MCP server, defined inline as this forge binary run as `forge mcp` in the project,
    ///   with `default_tools_approval_mode = "approve"`. Under `never`, an MCP call that needs approval
    ///   fails ("MCP tool call requires approval, but approval policy is never", the B1 Codex turn).
    ///   Only Forge's own server is approved; any other MCP server the user registered keeps its
    ///   own setting.
    ///
    /// Not used: `--dangerously-bypass-approvals-and-sandbox` (no sandbox at all; ruled out by
    /// EMMA-SOUL Dx3 30e57667) and `--approve-for-me` (it sends approvals to an automatic model
    /// review, and in the $0 mock check it did not let the MCP call through).
    /// `tests/codex/argv-check.sh` runs these exact flags through codex-cli 0.162.1.
    pub(crate) fn unattended_args(forge_exe: &Path, project_root: &Path) -> Vec<String> {
        let server = |key: &str, value: String| format!("mcp_servers.forge.{key}={value}");
        vec![
            "--sandbox".into(),
            UNATTENDED_SANDBOX.into(),
            "-c".into(),
            server("command", toml_basic_str(&forge_exe.to_string_lossy())),
            "-c".into(),
            server("args", r#"["mcp"]"#.into()),
            "-c".into(),
            server("cwd", toml_basic_str(&project_root.to_string_lossy())),
            "-c".into(),
            server("default_tools_approval_mode", r#""approve""#.into()),
        ]
    }

    /// The forge binary that the dispatched Codex starts as its `forge` MCP server: this one.
    fn forge_exe() -> std::path::PathBuf {
        std::env::current_exe().unwrap_or_else(|_| "forge".into())
    }

    /// Build the task prompt text (shared between headless and initial_input).
    pub(crate) fn task_prompt(task: &Task, task_type: &str) -> String {
        let criteria = if task.acceptance_criteria.is_empty() {
            String::new()
        } else {
            format!(
                "Acceptance criteria:\n{}\n\n",
                task.acceptance_criteria
                    .iter()
                    .map(|c| format!("- {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        match task_type {
            "verify" => format!(
                "You are working on task {id}: {title}\n\n\
                 This is a VERIFICATION task. You must ACTIVELY TEST the code, not just read it.\n\n\
                 Description: {desc}\n\n\
                 {criteria}\
                 Required verification steps:\n\
                 1. Run the project's test suite (e.g. `npm test`, `pytest`, `cargo test`)\n\
                 2. Run the type checker (e.g. `tsc --noEmit`, `mypy`, `cargo clippy`)\n\
                 3. Check that all new/modified files have corresponding test files\n\
                 4. Verify test mocks match production code shapes (interceptors, constructors, etc.)\n\
                 5. Run the linter (e.g. `eslint`, `ruff`, `cargo fmt --check`)\n\
                 6. If this task touches auth/routing/middleware, also run E2E tests\n\n\
                 Report your findings: what passed, what failed, what's missing.\n\
                 If tests fail or coverage is missing, report the specific failures.",
                id = task.id,
                title = task.title,
                desc = task.description,
            ),
            "review" => format!(
                "You are working on task {id}: {title}\n\n\
                 This is a REVIEW task. Analyze the code without making changes.\n\n\
                 Description: {desc}\n\n\
                 {criteria}\
                 Do NOT modify files — this is a read-only review.\n\
                 When done, summarize your findings.",
                id = task.id,
                title = task.title,
                desc = task.description,
            ),
            _ => format!(
                "Complete task {id}: {title}\n\nDescription: {desc}\n\n\
                 {criteria}\
                 IMPORTANT: Before marking this task complete:\n\
                 - Write unit tests for any new files you create\n\
                 - Run the existing test suite to verify nothing breaks\n\
                 - Run the type checker if the project has one\n\
                 - If you modify a file that has a corresponding .test file, update the test mocks\n\n\
                 When done, summarize what you did.",
                id = task.id,
                title = task.title,
                desc = task.description,
            ),
        }
    }
}

impl ToolAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
    }

    /// Extract Codex's `Context NN% left` (context REMAINING → used `100 - left`) only when the FULL
    /// statusline structural signature is present on a line: the model token (`gpt-`), the middle-dot
    /// `·` separators, AND both the `Context …% left` and `weekly …% left` gauges co-occurring. A
    /// bare `Context 5% left` diagnostic has no model token, no separators, and no weekly gauge →
    /// `None` (n/a) by construction. Case-insensitive.
    fn parse_ctx_pct(&self, statusline: &str) -> Option<u8> {
        for line in statusline.lines() {
            let lower = line.to_lowercase();
            let has_model = lower.contains("gpt-");
            let has_separator = line.contains('\u{00b7}'); // middle dot ·
            let has_context = lower.contains("context") && lower.contains("% left");
            let has_weekly = lower.contains("weekly");
            if has_model
                && has_separator
                && has_context
                && has_weekly
                && let Some(left) = super::pct_after_word_percent(&lower, "context")
            {
                return Some(100 - left);
            }
        }
        None
    }

    fn render_config(
        &self,
        state: &ForgeState,
        tasks: &[Task],
        project_root: &Path,
    ) -> anyhow::Result<()> {
        let assigned: Vec<_> = tasks
            .iter()
            .filter(|t| {
                t.assigned_to
                    .as_ref()
                    .is_some_and(|a| *a == AgentType::Codex)
            })
            .collect();

        let mut content = String::new();
        content.push_str("# AGENTS.md (Auto-generated by Forge Orchestrator)\n\n");
        content.push_str(&format!("**Project:** {}\n", state.project_name));
        content.push_str(&format!(
            "**Last updated:** {}\n\n",
            state.updated_at.format("%Y-%m-%d %H:%M UTC")
        ));
        content.push_str("---\n\n");

        if assigned.is_empty() {
            content.push_str("## Status\n\nNo tasks currently assigned to Codex.\n\n");
        } else {
            content.push_str("## Assigned Tasks\n\n");
            for task in &assigned {
                content.push_str(&format!(
                    "### {}: {}\n\n{}\n\n",
                    task.id, task.title, task.description
                ));
                if !task.acceptance_criteria.is_empty() {
                    content.push_str("**Acceptance Criteria:**\n");
                    for c in &task.acceptance_criteria {
                        content.push_str(&format!("- [ ] {c}\n"));
                    }
                    content.push('\n');
                }
                if !task.locked_files.is_empty() {
                    content.push_str("**Files you may modify:**\n");
                    for file in &task.locked_files {
                        content.push_str(&format!("- `{file}`\n"));
                    }
                    content.push('\n');
                }
            }
        }

        content.push_str("## Rules\n\n");
        content.push_str("- Complete tasks one at a time\n");
        content.push_str("- Do NOT modify files outside your assigned task scope\n");
        content.push_str("- Run tests before marking a task complete\n");
        content.push_str("- Check `.forge/state.json` for current orchestration state\n");

        std::fs::write(project_root.join("AGENTS.md"), content)?;
        Ok(())
    }

    fn build_command(
        &self,
        task: &Task,
        project_root: &Path,
        auth_mode: &str,
        permissions: &str,
    ) -> Command {
        let task_type = task.task_type.as_deref().unwrap_or("");
        let prompt = Self::task_prompt(task, task_type);

        let mut cmd = Command::new("codex");
        cmd.arg("exec");
        if permissions == "yolo" {
            cmd.args(Self::unattended_args(&Self::forge_exe(), project_root));
        }
        cmd.args(["--skip-git-repo-check", &prompt]);
        cmd.current_dir(project_root);

        if auth_mode == "subscription" {
            cmd.env_remove("OPENAI_API_KEY");
        }

        cmd
    }

    /// PTY mode: use `codex exec` with the prompt as a CLI argument.
    ///
    /// Root cause (Emma/CLX9): Codex's resolve_prompt() exits immediately
    /// with code 1 ("No prompt provided") when stdin.is_terminal() == true
    /// and no prompt arg is given. In a PTY, stdin IS a terminal, so bare
    /// `codex` (interactive TUI) crashes before initial_input can be typed.
    ///
    /// Fix: pass the prompt directly via `codex exec "prompt"`. This also
    /// eliminates the fragile ready-pattern + delayed-input dance.
    fn build_command_interactive(
        &self,
        task: &Task,
        project_root: &Path,
        auth_mode: &str,
        _permissions: &str,
    ) -> Command {
        let task_type = task.task_type.as_deref().unwrap_or("");
        let prompt = Self::task_prompt(task, task_type);

        let mut cmd = Command::new("codex");
        cmd.arg("exec");
        cmd.args(Self::unattended_args(&Self::forge_exe(), project_root));
        cmd.args(["--skip-git-repo-check", &prompt]);
        cmd.current_dir(project_root);

        if auth_mode == "subscription" {
            cmd.env_remove("OPENAI_API_KEY");
        }

        cmd
    }

    // No initial_input — prompt is passed as CLI arg to `codex exec`.
    // No ready_pattern — `codex exec` starts executing immediately.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(task_type: &str) -> Task {
        let mut task = Task::new("T-001", "Add login form", "Implement the login UI");
        task.task_type = Some(task_type.to_string());
        task.acceptance_criteria = vec!["Form validates email".to_string()];
        task
    }

    #[test]
    fn test_codex_verify_prompt() {
        let task = make_task("verify");
        let prompt = CodexAdapter::task_prompt(&task, "verify");
        assert!(prompt.contains("VERIFICATION task"));
        assert!(prompt.contains("ACTIVELY TEST"));
        assert!(prompt.contains("Run the project's test suite"));
    }

    #[test]
    fn test_codex_implement_prompt_has_test_requirement() {
        let task = make_task("implement");
        let prompt = CodexAdapter::task_prompt(&task, "implement");
        assert!(prompt.contains("Write unit tests"));
        assert!(prompt.contains("Run the existing test suite"));
    }

    #[test]
    fn test_codex_review_prompt_is_readonly() {
        let task = make_task("review");
        let prompt = CodexAdapter::task_prompt(&task, "review");
        assert!(prompt.contains("REVIEW task"));
        assert!(prompt.contains("Do NOT modify files"));
    }

    fn argv(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_unattended_argv_has_no_removed_or_bypass_flag() {
        let task = make_task("implement");
        let root = Path::new("/proj");
        for cmd in [
            CodexAdapter.build_command(&task, root, "api", "yolo"),
            CodexAdapter.build_command_interactive(&task, root, "api", "default"),
        ] {
            let a = argv(&cmd);
            assert_eq!(a[0], "exec");
            assert!(!a.iter().any(|x| x == "--full-auto"), "{a:?}");
            assert!(!a.iter().any(|x| x.starts_with("--dangerously")), "{a:?}");
            let i = a
                .iter()
                .position(|x| x == "--sandbox")
                .expect("sandbox flag");
            assert_eq!(a[i + 1], "workspace-write");
            assert!(a.contains(
                &r#"mcp_servers.forge.default_tools_approval_mode="approve""#.to_string()
            ));
            assert!(a.contains(&r#"mcp_servers.forge.args=["mcp"]"#.to_string()));
            assert!(a.contains(&r#"mcp_servers.forge.cwd="/proj""#.to_string()));
            assert_eq!(
                a.last().unwrap(),
                &CodexAdapter::task_prompt(&task, "implement")
            );
        }
    }

    #[test]
    fn test_restricted_headless_argv_approves_nothing() {
        let task = make_task("implement");
        let a = argv(&CodexAdapter.build_command(&task, Path::new("/proj"), "api", "default"));
        assert_eq!(a[..2], ["exec", "--skip-git-repo-check"]);
        assert_eq!(a.len(), 3, "{a:?}");
    }

    #[test]
    fn test_toml_basic_str_escapes_paths() {
        assert_eq!(toml_basic_str("/a b/forge"), r#""/a b/forge""#);
        assert_eq!(
            toml_basic_str(r"C:\Users\x\forge.exe"),
            r#""C:\\Users\\x\\forge.exe""#
        );
        assert_eq!(toml_basic_str("q\"t"), r#""q\"t""#);
        assert_eq!(toml_basic_str("t\tn"), r#""t\u0009n""#);
    }

    /// Writes the adapter's real argv for `tests/codex/argv-check.sh` (one JSON array per line).
    /// Runs only when FORGE_CODEX_ARGV_OUT names the output file.
    #[test]
    fn dump_codex_argv_for_argv_check() {
        let Ok(out) = std::env::var("FORGE_CODEX_ARGV_OUT") else {
            return;
        };
        let task = make_task("implement");
        let root = Path::new("/work");
        let lines: Vec<String> = [
            (
                "headless-yolo",
                CodexAdapter.build_command(&task, root, "api", "yolo"),
            ),
            (
                "headless-default",
                CodexAdapter.build_command(&task, root, "api", "default"),
            ),
            (
                "pty",
                CodexAdapter.build_command_interactive(&task, root, "api", "default"),
            ),
        ]
        .iter()
        .map(|(name, cmd)| serde_json::json!({"name": name, "argv": argv(cmd)}).to_string())
        .collect();
        std::fs::write(out, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn test_codex_default_prompt() {
        let task = make_task("something");
        let prompt = CodexAdapter::task_prompt(&task, "something");
        assert!(prompt.contains("T-001"));
        assert!(prompt.contains("Write unit tests"));
    }
}
