use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Build,
    Verify,
    Fix,
    Uat,
}

impl std::fmt::Display for TaskPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskPhase::Build => write!(f, "build"),
            TaskPhase::Verify => write!(f, "verify"),
            TaskPhase::Fix => write!(f, "fix"),
            TaskPhase::Uat => write!(f, "uat"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Assigned,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Claude,
    Codex,
    Gemini,
    Any,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::Claude => write!(f, "claude"),
            AgentType::Codex => write!(f, "codex"),
            AgentType::Gemini => write!(f, "gemini"),
            AgentType::Any => write!(f, "any"),
        }
    }
}

impl std::str::FromStr for AgentType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(AgentType::Claude),
            "codex" => Ok(AgentType::Codex),
            "gemini" => Ok(AgentType::Gemini),
            "any" => Ok(AgentType::Any),
            _ => Err(anyhow::anyhow!("Unknown agent type: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub assigned_to: Option<AgentType>,
    /// Task type hint for adapter strategy: design, implement, review, test, document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    pub depends_on: Vec<String>,
    pub locked_files: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Which plan generation pass created this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_version: Option<u32>,
    /// Parent task ID for subtasks (e.g., V-002's parent is T-002)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task: Option<String>,
    /// Lifecycle phase: build, verify, fix
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<TaskPhase>,
    /// Retry count for verify/fix loops (max 3 before flagging for human)
    #[serde(default)]
    pub retry_count: u32,
    /// Agent that verified this task (populated when a V-xxx subtask completes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<AgentType>,
    /// When this task was verified
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<DateTime<Utc>>,
}

impl Task {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            status: TaskStatus::Pending,
            assigned_to: None,
            task_type: None,
            depends_on: Vec::new(),
            locked_files: Vec::new(),
            acceptance_criteria: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            plan_version: None,
            parent_task: None,
            phase: None,
            retry_count: 0,
            verified_by: None,
            verified_at: None,
        }
    }

    pub fn is_blocked(&self, completed_tasks: &[String]) -> bool {
        self.depends_on
            .iter()
            .any(|dep| !completed_tasks.contains(dep))
    }

    /// Write task as a markdown file to .forge/tasks/
    pub fn write_to_file(&self, forge_dir: &Path) -> anyhow::Result<()> {
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir)?;

        let content = format!(
            "# {id}: {title}\n\n\
             **Status:** {status:?}\n\
             **Assigned to:** {agent}\n\
             **Type:** {task_type}\n\
             **Phase:** {phase}\n\
             **Parent:** {parent}\n\
             **Created:** {created}\n\
             **Updated:** {updated}\n\n\
             ## Description\n\n{description}\n\n\
             ## Dependencies\n\n{deps}\n\n\
             ## Locked Files\n\n{files}\n\n\
             ## Acceptance Criteria\n\n{criteria}\n",
            id = self.id,
            title = self.title,
            status = self.status,
            agent = self
                .assigned_to
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "unassigned".into()),
            task_type = self.task_type.as_deref().unwrap_or("—"),
            phase = self
                .phase
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "—".into()),
            parent = self.parent_task.as_deref().unwrap_or("—"),
            created = self.created_at.format("%Y-%m-%d %H:%M UTC"),
            updated = self.updated_at.format("%Y-%m-%d %H:%M UTC"),
            description = self.description,
            deps = if self.depends_on.is_empty() {
                "None".to_string()
            } else {
                self.depends_on
                    .iter()
                    .map(|d| format!("- {d}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            files = if self.locked_files.is_empty() {
                "None".to_string()
            } else {
                self.locked_files
                    .iter()
                    .map(|f| format!("- `{f}`"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            criteria = if self.acceptance_criteria.is_empty() {
                "None specified".to_string()
            } else {
                self.acceptance_criteria
                    .iter()
                    .map(|c| format!("- [ ] {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        );

        let path = tasks_dir.join(format!("{}.md", self.id));
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Read task from a JSON sidecar file
    pub fn read_from_json(forge_dir: &Path, task_id: &str) -> anyhow::Result<Self> {
        let path = forge_dir.join("tasks").join(format!("{task_id}.json"));
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Write task JSON sidecar for machine parsing
    pub fn write_json(&self, forge_dir: &Path) -> anyhow::Result<()> {
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir)?;
        let path = tasks_dir.join(format!("{}.json", self.id));
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Manage the collection of tasks in .forge/tasks/
pub struct TaskManager {
    forge_dir: PathBuf,
}

impl TaskManager {
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    pub fn create_task(&self, task: &Task) -> anyhow::Result<()> {
        task.write_to_file(&self.forge_dir)?;
        task.write_json(&self.forge_dir)?;
        Ok(())
    }

    pub fn list_tasks(&self) -> anyhow::Result<Vec<Task>> {
        let tasks_dir = self.forge_dir.join("tasks");
        if !tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        for entry in std::fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = std::fs::read_to_string(&path)?;
                if let Ok(task) = serde_json::from_str::<Task>(&content) {
                    tasks.push(task);
                }
            }
        }

        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    pub fn get_task(&self, task_id: &str) -> anyhow::Result<Task> {
        Task::read_from_json(&self.forge_dir, task_id)
    }

    pub fn update_task(&self, task: &Task) -> anyhow::Result<()> {
        self.create_task(task) // overwrite
    }

    pub fn get_completed_task_ids(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .list_tasks()?
            .into_iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id)
            .collect())
    }

    /// Find the highest existing task ID number and return the next one.
    /// If no tasks exist, returns 1.
    pub fn next_task_number(&self) -> anyhow::Result<u32> {
        let tasks_dir = self.forge_dir.join("tasks");
        if !tasks_dir.exists() {
            return Ok(1);
        }

        let mut max_id: u32 = 0;
        for entry in std::fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name
                .strip_prefix("T-")
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(num) = num_str.parse::<u32>()
            {
                max_id = max_id.max(num);
            }
        }

        Ok(max_id + 1)
    }

    /// Find the highest existing verify task ID number (V-NNN) and return the next one.
    pub fn next_verify_number(&self) -> anyhow::Result<u32> {
        let tasks_dir = self.forge_dir.join("tasks");
        if !tasks_dir.exists() {
            return Ok(1);
        }
        let mut max_id: u32 = 0;
        for entry in std::fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name
                .strip_prefix("V-")
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(num) = num_str.parse::<u32>()
            {
                max_id = max_id.max(num);
            }
        }
        Ok(max_id + 1)
    }

    /// Find the highest existing UAT task ID number (U-NNN) and return the next one.
    pub fn next_uat_number(&self) -> anyhow::Result<u32> {
        let tasks_dir = self.forge_dir.join("tasks");
        if !tasks_dir.exists() {
            return Ok(1);
        }
        let mut max_id: u32 = 0;
        for entry in std::fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name
                .strip_prefix("U-")
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(num) = num_str.parse::<u32>()
            {
                max_id = max_id.max(num);
            }
        }
        Ok(max_id + 1)
    }

    /// Generate UAT subtasks for completed user-facing T-xxx tasks.
    /// U-xxx tasks are human-only validation checkpoints — never dispatched to AI.
    pub fn generate_uat_subtasks(&self) -> anyhow::Result<Vec<Task>> {
        use crate::brain::rule_based::{generate_uat_criteria, is_user_facing};

        let tasks = self.list_tasks()?;
        let mut next_u = self.next_uat_number()?;
        let mut generated = Vec::new();

        for task in &tasks {
            // Only completed T-xxx build tasks (skip V-xxx, U-xxx, Fix)
            if task.status != TaskStatus::Completed {
                continue;
            }
            if task.phase == Some(TaskPhase::Verify)
                || task.phase == Some(TaskPhase::Uat)
                || task.phase == Some(TaskPhase::Fix)
            {
                continue;
            }
            // Must start with T- (skip V-, U- by ID prefix too)
            if !task.id.starts_with("T-") {
                continue;
            }

            // Skip non-user-facing tasks
            if !is_user_facing(&task.title, &task.description) {
                continue;
            }

            // Skip if already has a U-xxx child (idempotent)
            let has_uat = tasks.iter().any(|t| {
                t.parent_task.as_deref() == Some(&task.id) && t.phase == Some(TaskPhase::Uat)
            });
            if has_uat {
                continue;
            }

            // Find corresponding V-xxx if exists
            let verify_id = tasks
                .iter()
                .find(|t| {
                    t.parent_task.as_deref() == Some(&task.id) && t.phase == Some(TaskPhase::Verify)
                })
                .map(|t| t.id.clone());

            // Generate human-testable acceptance criteria
            let criteria = generate_uat_criteria(&task.title, &task.acceptance_criteria);

            let uat_id = format!("U-{next_u:03}");
            let mut uat_task = Task::new(
                &uat_id,
                format!("UAT: {}", task.title),
                format!(
                    "Human validation for task {}.\nVerify user-facing behavior meets expectations.",
                    task.id,
                ),
            );
            uat_task.parent_task = Some(task.id.clone());
            uat_task.phase = Some(TaskPhase::Uat);
            uat_task.task_type = Some("uat".to_string());
            uat_task.depends_on = vec![verify_id.unwrap_or_else(|| task.id.clone())];
            uat_task.assigned_to = None; // Human-only
            uat_task.acceptance_criteria = criteria;

            self.create_task(&uat_task)?;
            generated.push(uat_task);
            next_u += 1;
        }
        Ok(generated)
    }

    /// Generate verify subtasks for each completed implement/test task that lacks one.
    pub fn generate_verify_subtasks(&self) -> anyhow::Result<Vec<Task>> {
        let tasks = self.list_tasks()?;
        let mut next_v = self.next_verify_number()?;
        let mut generated = Vec::new();

        for task in &tasks {
            if task.status != TaskStatus::Completed {
                continue;
            }
            let task_type = task.task_type.as_deref().unwrap_or("");
            if !matches!(task_type, "implement" | "test" | "") {
                continue;
            }

            // Skip if already has a verify subtask
            let has_verify = tasks.iter().any(|t| {
                t.parent_task.as_deref() == Some(&task.id) && t.phase == Some(TaskPhase::Verify)
            });
            if has_verify {
                continue;
            }

            let verify_id = format!("V-{next_v:03}");
            let mut verify_task = Task::new(
                &verify_id,
                format!("Verify: {}", task.title),
                format!(
                    "VERIFY task {} by RUNNING tests, type checker, and linter.\n\
                     Do NOT just read the code — actively execute verification commands.\n\n\
                     Original task: {}\n\n\
                     Original criteria:\n{}\n\n\
                     Your job: Confirm these criteria are met by running the actual test suite, \
                     type checker, and linter. Report specific pass/fail results.",
                    task.id,
                    task.description,
                    task.acceptance_criteria
                        .iter()
                        .map(|c| format!("- {c}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            );
            verify_task.parent_task = Some(task.id.clone());
            verify_task.phase = Some(TaskPhase::Verify);
            verify_task.task_type = Some("verify".to_string());
            verify_task.depends_on = vec![task.id.clone()];
            verify_task.locked_files = task.locked_files.clone();

            // Enrich acceptance criteria with verification-specific checks
            let mut enriched: Vec<String> = task
                .acceptance_criteria
                .iter()
                .map(|c| format!("Verify: {}", c))
                .collect();
            enriched.push(
                "All project tests pass (run the test suite, don't just read code)".to_string(),
            );
            enriched.push(
                "Type checker passes with zero errors (e.g. tsc --noEmit, mypy, clippy)"
                    .to_string(),
            );
            enriched.push("No new lint warnings introduced".to_string());
            enriched.push(
                "All new files have corresponding test files with meaningful assertions"
                    .to_string(),
            );

            // If the task touches auth/routing/middleware, add cross-cutting checks
            let combined_text = format!(
                "{} {}",
                task.title.to_lowercase(),
                task.description.to_lowercase()
            );
            let cross_cutting_keywords = [
                "auth",
                "jwt",
                "login",
                "route",
                "guard",
                "middleware",
                "interceptor",
                "session",
                "token",
                "cors",
                "permission",
            ];
            if cross_cutting_keywords
                .iter()
                .any(|kw| combined_text.contains(kw))
            {
                enriched.push(
                    "E2E tests still pass (auth/routing changes are cross-cutting)".to_string(),
                );
                enriched.push(
                    "Test mocks and fixtures updated to match new auth/routing state".to_string(),
                );
            }

            // If the task touches UI/frontend, add Playwright-aware checks
            let ui_keywords = [
                "component",
                "page",
                "dashboard",
                "ui",
                "frontend",
                "render",
                "view",
                "layout",
                "button",
                "form",
                "modal",
                "dialog",
                "navigation",
                "sidebar",
            ];
            if ui_keywords.iter().any(|kw| combined_text.contains(kw)) {
                enriched.push(
                    "Playwright E2E tests pass with zero console errors (use page.on('console') and page.on('pageerror') to capture browser errors)".to_string(),
                );
                enriched.push(
                    "No uncaught exceptions or unhandled promise rejections in browser console"
                        .to_string(),
                );
            }

            verify_task.acceptance_criteria = enriched;

            // CRITICAL: Cross-agent verification — the verifier MUST be a
            // different agent than the builder. This is the Circular Validation
            // Trap defense: if the same agent builds and verifies, it will
            // repeat its own mistakes instead of catching them.
            let builder = task.assigned_to.clone().unwrap_or(AgentType::Claude);
            verify_task.assigned_to = Some(pick_different_agent(&builder));

            self.create_task(&verify_task)?;
            generated.push(verify_task);
            next_v += 1;
        }
        Ok(generated)
    }

    pub fn get_next_available(&self) -> anyhow::Result<Option<Task>> {
        let completed = self.get_completed_task_ids()?;
        Ok(self.list_tasks()?.into_iter().find(|t| {
            t.status == TaskStatus::Pending
                && !t.is_blocked(&completed)
                && t.phase != Some(TaskPhase::Uat)
        }))
    }

    /// Generate fix tasks for failed quality gates.
    /// Fix tasks have phase=Build and task_type="fix" so they get dispatched to agents
    /// but do NOT generate V-xxx verify subtasks (generate_verify_subtasks skips "fix").
    pub fn generate_gate_fix_tasks(
        &self,
        failed_gates: &[crate::core::quality_gate::GateResult],
    ) -> anyhow::Result<Vec<Task>> {
        let mut next_t = self.next_task_number()?;
        let mut generated = Vec::new();

        for gate in failed_gates {
            let task_id = format!("T-{next_t:03}");
            let stderr_tail = truncate_output(&gate.stderr, 1500);
            let stdout_tail = truncate_output(&gate.stdout, 1500);

            let mut fix_task = Task::new(
                &task_id,
                format!("Fix: {} failures", gate.gate_name),
                format!(
                    "The quality gate '{}' failed with exit code {}.\n\
                     Fix ALL errors reported below. Run the gate command to verify your fix.\n\n\
                     STDERR:\n{}\n\n\
                     STDOUT:\n{}",
                    gate.gate_name, gate.exit_code, stderr_tail, stdout_tail,
                ),
            );
            fix_task.phase = Some(TaskPhase::Build);
            fix_task.task_type = Some("fix".to_string());
            fix_task.acceptance_criteria = vec![
                format!("{} passes with exit code 0", gate.gate_name),
                "No regressions introduced by the fix".to_string(),
            ];
            fix_task.assigned_to = Some(AgentType::Claude);

            self.create_task(&fix_task)?;
            generated.push(fix_task);
            next_t += 1;
        }
        Ok(generated)
    }
}

/// Pick a different agent than the builder for cross-agent verification.
/// Ensures v.agent != t.agent — the Circular Validation Trap defense.
fn pick_different_agent(builder: &AgentType) -> AgentType {
    match builder {
        AgentType::Claude | AgentType::Any => AgentType::Codex,
        AgentType::Codex => AgentType::Claude,
        AgentType::Gemini => AgentType::Claude,
    }
}

/// Keep the last `max` characters of output for task descriptions.
fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("...truncated...\n{}", &s[s.len() - max..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_task_number_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        assert_eq!(mgr.next_task_number().unwrap(), 1);
    }

    #[test]
    fn test_next_task_number_sequential() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        // Create T-001, T-002, T-003
        for i in 1..=3 {
            let task = Task::new(format!("T-{i:03}"), "Test", "desc");
            mgr.create_task(&task).unwrap();
        }
        assert_eq!(mgr.next_task_number().unwrap(), 4);
    }

    #[test]
    fn test_next_task_number_finds_max_not_count() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        // Create T-001, T-005, T-017 (gaps)
        for i in [1, 5, 17] {
            let task = Task::new(format!("T-{i:03}"), "Test", "desc");
            mgr.create_task(&task).unwrap();
        }
        // Should return 18, not 4
        assert_eq!(mgr.next_task_number().unwrap(), 18);
    }

    #[test]
    fn test_next_task_number_ignores_non_task_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        let tasks_dir = tmp.path().join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        // Create a real task
        let task = Task::new("T-003", "Test", "desc");
        mgr.create_task(&task).unwrap();

        // Create non-task files that should be ignored
        std::fs::write(tasks_dir.join("archive.json"), "{}").unwrap();
        std::fs::write(tasks_dir.join("T-003.md"), "# markdown").unwrap();
        std::fs::write(tasks_dir.join("notes.txt"), "notes").unwrap();

        assert_eq!(mgr.next_task_number().unwrap(), 4);
    }

    #[test]
    fn test_plan_version_field() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Test", "desc");
        task.plan_version = Some(1);
        mgr.create_task(&task).unwrap();

        let loaded = mgr.get_task("T-001").unwrap();
        assert_eq!(loaded.plan_version, Some(1));
    }

    #[test]
    fn test_existing_tasks_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Simulate first plan: T-001 completed
        let mut task1 = Task::new("T-001", "Auth module", "Implement auth");
        task1.status = TaskStatus::Completed;
        task1.plan_version = Some(1);
        mgr.create_task(&task1).unwrap();

        // Simulate second plan: new task at T-002
        let mut task2 = Task::new("T-002", "Caching layer", "Add caching");
        task2.plan_version = Some(2);
        mgr.create_task(&task2).unwrap();

        // Verify T-001 is still completed and untouched
        let loaded_t1 = mgr.get_task("T-001").unwrap();
        assert_eq!(loaded_t1.status, TaskStatus::Completed);
        assert_eq!(loaded_t1.plan_version, Some(1));
        assert_eq!(loaded_t1.title, "Auth module");

        // Verify T-002 exists with version 2
        let loaded_t2 = mgr.get_task("T-002").unwrap();
        assert_eq!(loaded_t2.plan_version, Some(2));
    }

    #[test]
    fn test_task_phase_serialization_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("V-001", "Verify auth", "desc");
        task.phase = Some(TaskPhase::Verify);
        task.parent_task = Some("T-001".to_string());
        task.retry_count = 2;
        mgr.create_task(&task).unwrap();

        let loaded = mgr.get_task("V-001").unwrap();
        assert_eq!(loaded.phase, Some(TaskPhase::Verify));
        assert_eq!(loaded.parent_task, Some("T-001".to_string()));
        assert_eq!(loaded.retry_count, 2);
    }

    #[test]
    fn test_task_phase_display() {
        assert_eq!(TaskPhase::Build.to_string(), "build");
        assert_eq!(TaskPhase::Verify.to_string(), "verify");
        assert_eq!(TaskPhase::Fix.to_string(), "fix");
        assert_eq!(TaskPhase::Uat.to_string(), "uat");
    }

    #[test]
    fn test_task_phase_uat_display() {
        assert_eq!(TaskPhase::Uat.to_string(), "uat");
    }

    #[test]
    fn test_next_uat_number_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        assert_eq!(mgr.next_uat_number().unwrap(), 1);
    }

    #[test]
    fn test_next_uat_number_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        for i in [1, 3] {
            let mut task = Task::new(format!("U-{i:03}"), "UAT task", "desc");
            task.phase = Some(TaskPhase::Uat);
            mgr.create_task(&task).unwrap();
        }
        assert_eq!(mgr.next_uat_number().unwrap(), 4);
    }

    #[test]
    fn test_generate_uat_for_ui_task() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new(
            "T-001",
            "Add dashboard sidebar",
            "Create navigation sidebar component",
        );
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.phase = Some(TaskPhase::Build);
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_uat_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].id, "U-001");
        assert_eq!(generated[0].parent_task, Some("T-001".to_string()));
        assert_eq!(generated[0].phase, Some(TaskPhase::Uat));
        assert_eq!(generated[0].task_type, Some("uat".to_string()));
        assert!(generated[0].assigned_to.is_none()); // Human-only
        assert!(!generated[0].acceptance_criteria.is_empty());
    }

    #[test]
    fn test_generate_uat_skips_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new(
            "T-001",
            "Refactor database module",
            "Clean up internal SQL layer",
        );
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.phase = Some(TaskPhase::Build);
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_uat_subtasks().unwrap();
        assert_eq!(generated.len(), 0); // Internal/infra task — no UAT
    }

    #[test]
    fn test_generate_uat_depends_on_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Create T-001 (completed build task with UI keyword)
        let mut t1 = Task::new("T-001", "Add dashboard layout", "Create main layout");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        t1.phase = Some(TaskPhase::Build);
        mgr.create_task(&t1).unwrap();

        // Create V-001 (verify subtask of T-001)
        let mut v1 = Task::new("V-001", "Verify: Add dashboard layout", "desc");
        v1.phase = Some(TaskPhase::Verify);
        v1.parent_task = Some("T-001".to_string());
        v1.status = TaskStatus::Completed;
        mgr.create_task(&v1).unwrap();

        let generated = mgr.generate_uat_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].depends_on, vec!["V-001".to_string()]);
    }

    #[test]
    fn test_generate_uat_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Add sidebar component", "UI sidebar");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.phase = Some(TaskPhase::Build);
        mgr.create_task(&task).unwrap();

        let gen1 = mgr.generate_uat_subtasks().unwrap();
        assert_eq!(gen1.len(), 1);

        let gen2 = mgr.generate_uat_subtasks().unwrap();
        assert_eq!(gen2.len(), 0); // Idempotent
    }

    #[test]
    fn test_retry_count_defaults_to_zero() {
        let task = Task::new("T-001", "Test", "desc");
        assert_eq!(task.retry_count, 0);
        assert!(task.parent_task.is_none());
        assert!(task.phase.is_none());
    }

    #[test]
    fn test_next_verify_number_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        assert_eq!(mgr.next_verify_number().unwrap(), 1);
    }

    #[test]
    fn test_next_verify_number_with_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());
        for i in [1, 3] {
            let mut task = Task::new(format!("V-{i:03}"), "Verify", "desc");
            task.phase = Some(TaskPhase::Verify);
            mgr.create_task(&task).unwrap();
        }
        assert_eq!(mgr.next_verify_number().unwrap(), 4);
    }

    #[test]
    fn test_generate_verify_subtasks_for_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Auth module", "Implement auth");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].id, "V-001");
        assert_eq!(generated[0].parent_task, Some("T-001".to_string()));
        assert_eq!(generated[0].phase, Some(TaskPhase::Verify));
        assert_eq!(generated[0].assigned_to, Some(AgentType::Codex));
    }

    #[test]
    fn test_generate_verify_subtasks_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Auth module", "Implement auth");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        mgr.create_task(&task).unwrap();

        // First call generates
        let gen1 = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(gen1.len(), 1);

        // Second call is idempotent
        let gen2 = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(gen2.len(), 0);
    }

    #[test]
    fn test_generate_verify_skips_non_implement() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Write docs", "docs");
        task.task_type = Some("document".to_string());
        task.status = TaskStatus::Completed;
        mgr.create_task(&task).unwrap();

        // "document" tasks should NOT get verify subtasks
        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 0);
    }

    #[test]
    fn test_generate_verify_agent_assignment() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Unassigned task defaults to Claude → verifier must be Codex (different agent)
        let mut t1 = Task::new("T-001", "Default task", "desc");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        mgr.create_task(&t1).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated[0].assigned_to, Some(AgentType::Codex));
    }

    #[test]
    fn test_cross_agent_verification_never_same_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Codex built it → Claude must verify (v.agent != t.agent)
        let mut t1 = Task::new("T-001", "Codex task", "desc");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        t1.assigned_to = Some(AgentType::Codex);
        mgr.create_task(&t1).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].assigned_to, Some(AgentType::Claude));
        assert_ne!(
            generated[0].assigned_to, t1.assigned_to,
            "Verifier must differ from builder"
        );
    }

    #[test]
    fn test_cross_agent_verification_gemini_builder() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Gemini built it → Claude must verify
        let mut t1 = Task::new("T-001", "Gemini task", "desc");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        t1.assigned_to = Some(AgentType::Gemini);
        mgr.create_task(&t1).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated[0].assigned_to, Some(AgentType::Claude));
        assert_ne!(generated[0].assigned_to, t1.assigned_to);
    }

    #[test]
    fn test_cross_agent_verification_claude_builder() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        // Claude built it → Codex must verify
        let mut t1 = Task::new("T-001", "Claude task", "desc");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        t1.assigned_to = Some(AgentType::Claude);
        mgr.create_task(&t1).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated[0].assigned_to, Some(AgentType::Codex));
        assert_ne!(generated[0].assigned_to, t1.assigned_to);
    }

    #[test]
    fn test_generate_verify_enriched_criteria() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Add caching layer", "Implement Redis cache");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec!["Cache invalidation works".to_string()];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        assert!(
            criteria
                .iter()
                .any(|c| c.contains("All project tests pass"))
        );
        assert!(criteria.iter().any(|c| c.contains("Type checker passes")));
        assert!(criteria.iter().any(|c| c.contains("No new lint warnings")));
        assert!(
            criteria
                .iter()
                .any(|c| c.contains("corresponding test files"))
        );
    }

    #[test]
    fn test_generate_verify_cross_cutting_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "JWT auth middleware", "Add JWT token validation");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec!["Tokens validated correctly".to_string()];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        assert!(criteria.iter().any(|c| c.contains("E2E tests still pass")));
        assert!(
            criteria
                .iter()
                .any(|c| c.contains("Test mocks and fixtures updated"))
        );
    }

    #[test]
    fn test_generate_verify_non_cross_cutting() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new(
            "T-001",
            "Optimize query performance",
            "Speed up SQL queries",
        );
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec!["Query runs under 100ms".to_string()];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        // Should NOT have cross-cutting criteria
        assert!(!criteria.iter().any(|c| c.contains("E2E tests still pass")));
        assert!(
            !criteria
                .iter()
                .any(|c| c.contains("Test mocks and fixtures"))
        );
    }

    #[test]
    fn test_generate_verify_prefixed_criteria() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new("T-001", "Build API endpoint", "Create REST endpoint");
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec![
            "Returns 200 on success".to_string(),
            "Returns 404 for missing resource".to_string(),
        ];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        assert!(
            criteria
                .iter()
                .any(|c| c == "Verify: Returns 200 on success")
        );
        assert!(
            criteria
                .iter()
                .any(|c| c == "Verify: Returns 404 for missing resource")
        );
    }

    #[test]
    fn test_generate_gate_fix_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let failed = vec![crate::core::quality_gate::GateResult {
            gate_name: "TypeCheck".to_string(),
            passed: false,
            exit_code: 2,
            stdout: String::new(),
            stderr: "TS2345: Argument of type 'string' is not assignable".to_string(),
            duration_ms: 500,
        }];

        let generated = mgr.generate_gate_fix_tasks(&failed).unwrap();
        assert_eq!(generated.len(), 1);
        let fix = &generated[0];
        assert_eq!(fix.title, "Fix: TypeCheck failures");
        assert_eq!(fix.phase, Some(TaskPhase::Build));
        assert_eq!(fix.task_type.as_deref(), Some("fix"));
        assert_eq!(fix.assigned_to, Some(AgentType::Claude));
        assert!(fix.description.contains("TS2345"));
        assert!(fix.acceptance_criteria[0].contains("TypeCheck passes"));
    }

    #[test]
    fn test_gate_fix_task_multiple_gates() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let failed = vec![
            crate::core::quality_gate::GateResult {
                gate_name: "TypeCheck".to_string(),
                passed: false,
                exit_code: 2,
                stdout: String::new(),
                stderr: "type error".to_string(),
                duration_ms: 100,
            },
            crate::core::quality_gate::GateResult {
                gate_name: "Lint".to_string(),
                passed: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: "unused import".to_string(),
                duration_ms: 50,
            },
        ];

        let generated = mgr.generate_gate_fix_tasks(&failed).unwrap();
        assert_eq!(generated.len(), 2);
        assert_eq!(generated[0].title, "Fix: TypeCheck failures");
        assert_eq!(generated[1].title, "Fix: Lint failures");
        // Should have sequential task IDs
        assert_eq!(generated[0].id, "T-001");
        assert_eq!(generated[1].id, "T-002");
    }

    #[test]
    fn test_gate_fix_task_not_verified() {
        // Fix tasks with task_type="fix" should NOT generate verify subtasks
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let failed = vec![crate::core::quality_gate::GateResult {
            gate_name: "Test Suite".to_string(),
            passed: false,
            exit_code: 1,
            stdout: "FAIL src/foo.test.ts".to_string(),
            stderr: String::new(),
            duration_ms: 200,
        }];

        let generated = mgr.generate_gate_fix_tasks(&failed).unwrap();
        assert_eq!(generated.len(), 1);

        // Mark fix task as completed
        let mut fix = generated[0].clone();
        fix.status = TaskStatus::Completed;
        mgr.update_task(&fix).unwrap();

        // generate_verify_subtasks should skip "fix" type tasks
        let verify = mgr.generate_verify_subtasks().unwrap();
        assert!(verify.is_empty(), "fix tasks should NOT generate V-xxx");
    }

    #[test]
    fn test_generate_verify_ui_task_has_playwright_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new(
            "T-001",
            "Build dashboard component",
            "Create a React dashboard page with charts",
        );
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec!["Dashboard renders charts".to_string()];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        assert!(
            criteria
                .iter()
                .any(|c| c.contains("Playwright") && c.contains("console errors")),
            "UI tasks should have Playwright console error check, got: {criteria:?}"
        );
        assert!(
            criteria.iter().any(|c| c.contains("uncaught exceptions")),
            "UI tasks should check for uncaught exceptions"
        );
    }

    #[test]
    fn test_generate_verify_non_ui_task_no_playwright() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = TaskManager::new(tmp.path());

        let mut task = Task::new(
            "T-001",
            "Optimize database queries",
            "Speed up the batch processing pipeline",
        );
        task.task_type = Some("implement".to_string());
        task.status = TaskStatus::Completed;
        task.acceptance_criteria = vec!["Queries under 100ms".to_string()];
        mgr.create_task(&task).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated.len(), 1);
        let criteria = &generated[0].acceptance_criteria;
        assert!(
            !criteria.iter().any(|c| c.contains("Playwright")),
            "Non-UI tasks should NOT have Playwright checks"
        );
    }
}
