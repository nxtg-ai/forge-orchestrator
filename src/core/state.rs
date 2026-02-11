use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::task::AgentType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeState {
    pub version: String,
    pub project_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tools: Vec<DetectedTool>,
    pub brain: BrainConfig,
    pub task_summary: TaskSummary,
    pub active_locks: HashMap<String, FileLock>,
    /// Per-agent auth mode: "subscription" strips API keys, "api" passes them through
    #[serde(default)]
    pub agent_auth: HashMap<String, String>,
    /// Per-agent permission mode: "safe" (read-only) or "yolo" (full autonomy)
    #[serde(default)]
    pub agent_permissions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTool {
    pub name: String,
    pub agent_type: AgentType,
    pub version: Option<String>,
    pub path: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub blocked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    pub task_id: String,
    pub agent: AgentType,
    pub locked_at: DateTime<Utc>,
    pub files: Vec<String>,
}

impl Default for ForgeState {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            version: "0.1.0".into(),
            project_name: String::new(),
            created_at: now,
            updated_at: now,
            tools: Vec::new(),
            brain: BrainConfig {
                provider: "rule-based".into(),
                model: None,
            },
            task_summary: TaskSummary::default(),
            active_locks: HashMap::new(),
            agent_auth: HashMap::new(),
            agent_permissions: HashMap::new(),
        }
    }
}

/// Manages .forge/state.json — the fast-read cache of orchestration state
pub struct StateManager {
    forge_dir: PathBuf,
}

impl StateManager {
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.forge_dir.join("state.json")
    }

    pub fn load(&self) -> anyhow::Result<ForgeState> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(ForgeState::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self, state: &ForgeState) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.forge_dir)?;
        let content = serde_json::to_string_pretty(state)?;
        std::fs::write(self.state_path(), content)?;
        Ok(())
    }

    pub fn update_task_summary(&self, summary: TaskSummary) -> anyhow::Result<()> {
        let mut state = self.load()?;
        state.task_summary = summary;
        state.updated_at = Utc::now();
        self.save(&state)
    }

    /// Recompute task_summary from actual task files on disk
    pub fn refresh_task_summary(&self) -> anyhow::Result<()> {
        let task_mgr = super::task::TaskManager::new(&self.forge_dir);
        let tasks = task_mgr.list_tasks()?;
        let summary = TaskSummary {
            total: tasks.len(),
            pending: tasks
                .iter()
                .filter(|t| t.status == super::task::TaskStatus::Pending)
                .count(),
            in_progress: tasks
                .iter()
                .filter(|t| {
                    t.status == super::task::TaskStatus::Assigned
                        || t.status == super::task::TaskStatus::InProgress
                })
                .count(),
            completed: tasks
                .iter()
                .filter(|t| t.status == super::task::TaskStatus::Completed)
                .count(),
            failed: tasks
                .iter()
                .filter(|t| t.status == super::task::TaskStatus::Failed)
                .count(),
            blocked: tasks
                .iter()
                .filter(|t| t.status == super::task::TaskStatus::Blocked)
                .count(),
        };
        self.update_task_summary(summary)
    }

    /// Refresh detected tools in state (picks up newly installed CLIs)
    pub fn update_tools(&self, tools: &[DetectedTool]) -> anyhow::Result<()> {
        let mut state = self.load()?;
        state.tools = tools.to_vec();
        state.updated_at = Utc::now();
        self.save(&state)
    }

    /// Check if any locked files conflict with a set of files
    pub fn check_file_conflicts(&self, files: &[String]) -> anyhow::Result<Vec<FileLock>> {
        let state = self.load()?;
        let conflicts: Vec<FileLock> = state
            .active_locks
            .values()
            .filter(|lock| lock.files.iter().any(|f| files.contains(f)))
            .cloned()
            .collect();
        Ok(conflicts)
    }

    /// Lock files for a task
    pub fn lock_files(
        &self,
        task_id: &str,
        agent: AgentType,
        files: Vec<String>,
    ) -> anyhow::Result<()> {
        let mut state = self.load()?;
        state.active_locks.insert(
            task_id.to_string(),
            FileLock {
                task_id: task_id.to_string(),
                agent,
                locked_at: Utc::now(),
                files,
            },
        );
        state.updated_at = Utc::now();
        self.save(&state)
    }

    /// Get the auth mode for an agent ("subscription" or "api"). Defaults to "subscription".
    pub fn get_agent_auth(&self, agent: &str) -> anyhow::Result<String> {
        let state = self.load()?;
        Ok(state
            .agent_auth
            .get(agent)
            .cloned()
            .unwrap_or_else(|| "subscription".to_string()))
    }

    /// Get the permission mode for an agent ("safe" or "yolo"). Defaults to "safe".
    pub fn get_agent_permissions(&self, agent: &str) -> anyhow::Result<String> {
        let state = self.load()?;
        Ok(state
            .agent_permissions
            .get(agent)
            .cloned()
            .unwrap_or_else(|| "safe".to_string()))
    }

    /// Unlock files for a completed task
    pub fn unlock_files(&self, task_id: &str) -> anyhow::Result<()> {
        let mut state = self.load()?;
        state.active_locks.remove(task_id);
        state.updated_at = Utc::now();
        self.save(&state)
    }
}
