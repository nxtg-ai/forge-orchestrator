use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    pub fn get_next_available(&self) -> anyhow::Result<Option<Task>> {
        let completed = self.get_completed_task_ids()?;
        Ok(self
            .list_tasks()?
            .into_iter()
            .find(|t| t.status == TaskStatus::Pending && !t.is_blocked(&completed)))
    }
}
