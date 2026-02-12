use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Build,
    Verify,
    Fix,
}

impl std::fmt::Display for TaskPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskPhase::Build => write!(f, "build"),
            TaskPhase::Verify => write!(f, "verify"),
            TaskPhase::Fix => write!(f, "fix"),
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
                t.parent_task.as_deref() == Some(&task.id)
                    && t.phase == Some(TaskPhase::Verify)
            });
            if has_verify {
                continue;
            }

            let verify_id = format!("V-{next_v:03}");
            let mut verify_task = Task::new(
                &verify_id,
                format!("Verify: {}", task.title),
                format!(
                    "Run automated tests for task {}.\nReview changes and verify acceptance criteria.\nOriginal: {}\nCriteria:\n{}",
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
            verify_task.task_type = Some("review".to_string());
            verify_task.depends_on = vec![task.id.clone()];
            verify_task.locked_files = task.locked_files.clone();
            verify_task.acceptance_criteria = task.acceptance_criteria.clone();
            verify_task.assigned_to = Some(match task_type {
                "document" => AgentType::Gemini,
                "design" => AgentType::Claude,
                _ => AgentType::Codex,
            });

            self.create_task(&verify_task)?;
            generated.push(verify_task);
            next_v += 1;
        }
        Ok(generated)
    }

    pub fn get_next_available(&self) -> anyhow::Result<Option<Task>> {
        let completed = self.get_completed_task_ids()?;
        Ok(self
            .list_tasks()?
            .into_iter()
            .find(|t| t.status == TaskStatus::Pending && !t.is_blocked(&completed)))
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

        // Default type → Codex
        let mut t1 = Task::new("T-001", "Default task", "desc");
        t1.task_type = Some("implement".to_string());
        t1.status = TaskStatus::Completed;
        mgr.create_task(&t1).unwrap();

        let generated = mgr.generate_verify_subtasks().unwrap();
        assert_eq!(generated[0].assigned_to, Some(AgentType::Codex));
    }
}
