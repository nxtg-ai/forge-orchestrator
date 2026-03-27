use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::task::AgentType;

/// Root orchestration state persisted to `.forge/state.json`.
///
/// This is the fast-read cache of project metadata, detected tools,
/// task summaries, file locks, and configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeState {
    /// Schema version of the state file format.
    pub version: String,
    /// Human-readable project name.
    pub project_name: String,
    /// When this project was initialized with `forge init`.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp (updated on every state write).
    pub updated_at: DateTime<Utc>,
    /// AI tools detected on the system PATH (claude, codex, gemini).
    pub tools: Vec<DetectedTool>,
    /// LLM brain configuration (provider + optional model).
    pub brain: BrainConfig,
    /// Cached task counts by status, refreshed from task files on disk.
    pub task_summary: TaskSummary,
    /// Currently locked files keyed by task ID, preventing concurrent edits.
    pub active_locks: HashMap<String, FileLock>,
    /// Per-agent auth mode: "subscription" strips API keys, "api" passes them through
    #[serde(default)]
    pub agent_auth: HashMap<String, String>,
    /// Per-agent permission mode: "safe" (read-only) or "yolo" (full autonomy)
    #[serde(default)]
    pub agent_permissions: HashMap<String, String>,
    /// Git integration config
    #[serde(default)]
    pub git: GitConfig,
    /// Scheduler config (pacing, rotation)
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    /// Dashboard rendering mode: "pty" (Stargate) or "piped" (legacy). Default: "piped".
    #[serde(default = "default_dashboard_mode")]
    pub dashboard_mode: String,
    /// Current dashboard phase: "build", "verify", "complete". Updated in real-time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<String>,
}

fn default_dashboard_mode() -> String {
    "piped".to_string()
}

/// An AI tool binary detected on the system PATH during `forge init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTool {
    /// CLI binary name (e.g., `"claude"`, `"codex"`, `"gemini"`).
    pub name: String,
    /// Corresponding agent type enum value.
    pub agent_type: AgentType,
    /// Version string if detectable (e.g., `"1.0.39"`).
    pub version: Option<String>,
    /// Absolute path to the binary on disk.
    pub path: String,
    /// Whether the binary is currently usable.
    pub available: bool,
}

/// Configuration for the pluggable LLM brain backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    /// Provider name: `"rule-based"` (free heuristic) or `"openai"` (API-powered).
    pub provider: String,
    /// Optional model identifier (e.g., `"gpt-4o"`, `"o3"`).
    pub model: Option<String>,
}

/// Aggregated task counts by status, cached in state for fast reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskSummary {
    /// Total number of tasks across all statuses.
    pub total: usize,
    /// Tasks not yet assigned or started.
    pub pending: usize,
    /// Tasks currently assigned or being worked on.
    pub in_progress: usize,
    /// Tasks that finished successfully.
    pub completed: usize,
    /// Tasks that failed execution.
    pub failed: usize,
    /// Tasks waiting on unmet dependencies.
    pub blocked: usize,
}

/// An exclusive file lock held by a task to prevent concurrent edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    /// The task ID that owns this lock.
    pub task_id: String,
    /// The agent type that claimed the lock.
    pub agent: AgentType,
    /// When the lock was acquired.
    pub locked_at: DateTime<Utc>,
    /// File paths under exclusive access.
    pub files: Vec<String>,
}

/// Git integration settings controlling auto-commit behavior after task completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Enable auto-commit after each task completes. Default: true.
    pub auto_commit: bool,
    /// Git strategy: "single" (commit to current branch). Future: "worktree", "branch".
    pub strategy: String,
}

/// Scheduler settings for provider rotation and rate-limit pacing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Enable provider rotation when a provider is rate-limited. Default: false.
    pub rotation: bool,
    /// Minimum pacing delay in seconds for subscription mode. Default: 64.
    pub pacing_min_secs: u64,
    /// Maximum pacing delay in seconds for subscription mode. Default: 179.
    pub pacing_max_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            rotation: false,
            pacing_min_secs: 64,
            pacing_max_secs: 179,
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            auto_commit: true,
            strategy: "single".to_string(),
        }
    }
}

impl Default for ForgeState {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            version: "1.1.0".into(),
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
            git: GitConfig::default(),
            scheduler: SchedulerConfig::default(),
            dashboard_mode: default_dashboard_mode(),
            current_phase: None,
        }
    }
}

/// Manages `.forge/state.json` — the fast-read cache of orchestration state.
///
/// All reads and writes go through this manager to ensure consistent
/// serialization and automatic `updated_at` timestamp maintenance.
pub struct StateManager {
    /// Path to the `.forge/` directory.
    forge_dir: PathBuf,
}

impl StateManager {
    /// Create a new `StateManager` targeting the given `.forge/` directory.
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.forge_dir.join("state.json")
    }

    /// Load the current state from disk, returning defaults if the file does not exist.
    pub fn load(&self) -> anyhow::Result<ForgeState> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(ForgeState::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Persist the given state to disk as pretty-printed JSON.
    pub fn save(&self, state: &ForgeState) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.forge_dir)?;
        let content = serde_json::to_string_pretty(state)?;
        std::fs::write(self.state_path(), content)?;
        Ok(())
    }

    /// Replace the cached task summary and bump `updated_at`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::{AgentType, Task, TaskStatus};

    // ---------------------------------------------------------------
    // Existing tests (preserved)
    // ---------------------------------------------------------------

    #[test]
    fn test_git_config_default() {
        let cfg = GitConfig::default();
        assert!(cfg.auto_commit);
        assert_eq!(cfg.strategy, "single");
    }

    #[test]
    fn test_forge_state_default_has_git() {
        let state = ForgeState::default();
        assert!(state.git.auto_commit);
        assert_eq!(state.git.strategy, "single");
    }

    #[test]
    fn test_state_without_git_field_deserializes() {
        // Simulate old state.json without the git field
        let json = r#"{
            "version": "0.2.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "tools": [],
            "brain": { "provider": "rule-based", "model": null },
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "active_locks": {}
        }"#;
        let state: ForgeState = serde_json::from_str(json).unwrap();
        // Should use defaults for missing fields
        assert!(state.git.auto_commit);
        assert_eq!(state.git.strategy, "single");
    }

    #[test]
    fn test_config_roundtrip_with_git() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state.git.auto_commit = false;
        mgr.save(&state).unwrap();

        let loaded = mgr.load().unwrap();
        assert!(!loaded.git.auto_commit);
    }

    #[test]
    fn test_dashboard_mode_default() {
        let state = ForgeState::default();
        assert_eq!(state.dashboard_mode, "piped");
    }

    #[test]
    fn test_dashboard_mode_backward_compat() {
        // Old state.json without dashboard_mode field should deserialize fine
        let json = r#"{
            "version": "1.2.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "tools": [],
            "brain": { "provider": "rule-based", "model": null },
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "active_locks": {}
        }"#;
        let state: ForgeState = serde_json::from_str(json).unwrap();
        assert_eq!(state.dashboard_mode, "piped");
    }

    // ---------------------------------------------------------------
    // 1. load returns Default when no file exists — assert each field
    // ---------------------------------------------------------------

    #[test]
    fn test_load_no_file_returns_default_values() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let state = mgr.load().unwrap();

        assert_eq!(state.project_name, "");
        assert_eq!(state.version, "1.1.0");
        assert_eq!(state.task_summary.total, 0);
        assert_eq!(state.task_summary.pending, 0);
        assert_eq!(state.task_summary.in_progress, 0);
        assert_eq!(state.task_summary.completed, 0);
        assert_eq!(state.task_summary.failed, 0);
        assert_eq!(state.task_summary.blocked, 0);
        assert!(state.active_locks.is_empty());
        assert!(state.agent_auth.is_empty());
        assert!(state.agent_permissions.is_empty());
        assert!(state.tools.is_empty());
        assert_eq!(state.brain.provider, "rule-based");
        assert!(state.brain.model.is_none());
        assert_eq!(state.dashboard_mode, "piped");
    }

    // ---------------------------------------------------------------
    // 2. save + load roundtrip — ALL fields survive with non-defaults
    // ---------------------------------------------------------------

    #[test]
    fn test_save_load_roundtrip_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state.project_name = "roundtrip-project".to_string();
        state.version = "2.0.0".to_string();
        state.task_summary = TaskSummary {
            total: 10,
            pending: 3,
            in_progress: 2,
            completed: 4,
            failed: 1,
            blocked: 0,
        };
        state.brain = BrainConfig {
            provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
        };
        state.git = GitConfig {
            auto_commit: false,
            strategy: "worktree".to_string(),
        };
        state.scheduler = SchedulerConfig {
            rotation: true,
            pacing_min_secs: 30,
            pacing_max_secs: 120,
        };
        state.dashboard_mode = "pty".to_string();
        state
            .agent_auth
            .insert("claude".to_string(), "api".to_string());
        state
            .agent_permissions
            .insert("codex".to_string(), "yolo".to_string());

        mgr.save(&state).unwrap();
        let loaded = mgr.load().unwrap();

        assert_eq!(loaded.project_name, "roundtrip-project");
        assert_eq!(loaded.version, "2.0.0");
        assert_eq!(loaded.task_summary.total, 10);
        assert_eq!(loaded.task_summary.pending, 3);
        assert_eq!(loaded.task_summary.in_progress, 2);
        assert_eq!(loaded.task_summary.completed, 4);
        assert_eq!(loaded.task_summary.failed, 1);
        assert_eq!(loaded.task_summary.blocked, 0);
        assert_eq!(loaded.brain.provider, "openai");
        assert_eq!(loaded.brain.model.as_deref(), Some("gpt-5"));
        assert!(!loaded.git.auto_commit);
        assert_eq!(loaded.git.strategy, "worktree");
        assert!(loaded.scheduler.rotation);
        assert_eq!(loaded.scheduler.pacing_min_secs, 30);
        assert_eq!(loaded.scheduler.pacing_max_secs, 120);
        assert_eq!(loaded.dashboard_mode, "pty");
        assert_eq!(loaded.agent_auth.get("claude").unwrap(), "api");
        assert_eq!(loaded.agent_permissions.get("codex").unwrap(), "yolo");
    }

    // ---------------------------------------------------------------
    // 3. update_task_summary — assert each field and updated_at
    // ---------------------------------------------------------------

    #[test]
    fn test_update_task_summary_persists_each_field() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        // Save initial state so state.json exists
        let initial = ForgeState::default();
        let created_at = initial.created_at;
        mgr.save(&initial).unwrap();

        let summary = TaskSummary {
            total: 5,
            pending: 2,
            in_progress: 0,
            completed: 3,
            failed: 0,
            blocked: 0,
        };
        mgr.update_task_summary(summary).unwrap();

        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.task_summary.total, 5);
        assert_eq!(loaded.task_summary.pending, 2);
        assert_eq!(loaded.task_summary.in_progress, 0);
        assert_eq!(loaded.task_summary.completed, 3);
        assert_eq!(loaded.task_summary.failed, 0);
        assert_eq!(loaded.task_summary.blocked, 0);
        // updated_at should be >= created_at (it was refreshed)
        assert!(loaded.updated_at >= created_at);
    }

    #[test]
    fn test_update_task_summary_with_all_statuses() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let summary = TaskSummary {
            total: 15,
            pending: 3,
            in_progress: 4,
            completed: 5,
            failed: 2,
            blocked: 1,
        };
        mgr.update_task_summary(summary).unwrap();

        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.task_summary.total, 15);
        assert_eq!(loaded.task_summary.pending, 3);
        assert_eq!(loaded.task_summary.in_progress, 4);
        assert_eq!(loaded.task_summary.completed, 5);
        assert_eq!(loaded.task_summary.failed, 2);
        assert_eq!(loaded.task_summary.blocked, 1);
    }

    // ---------------------------------------------------------------
    // 4. lock_files + check_file_conflicts
    // ---------------------------------------------------------------

    #[test]
    fn test_lock_files_and_check_conflicts_single_task() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        // Lock two files for task-1
        mgr.lock_files(
            "task-1",
            AgentType::Claude,
            vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        )
        .unwrap();

        // Overlapping file should produce 1 conflict
        let conflicts = mgr
            .check_file_conflicts(&["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].task_id, "task-1");
        assert_eq!(conflicts[0].files.len(), 2);
        assert!(conflicts[0].files.contains(&"src/main.rs".to_string()));
        assert!(conflicts[0].files.contains(&"src/lib.rs".to_string()));

        // Non-overlapping file should produce 0 conflicts
        let no_conflicts = mgr
            .check_file_conflicts(&["src/other.rs".to_string()])
            .unwrap();
        assert_eq!(no_conflicts.len(), 0);
    }

    #[test]
    fn test_lock_files_multiple_tasks_overlapping() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["src/main.rs".to_string()])
            .unwrap();

        mgr.lock_files(
            "task-2",
            AgentType::Codex,
            vec!["src/main.rs".to_string(), "src/util.rs".to_string()],
        )
        .unwrap();

        // Both tasks lock src/main.rs — should return 2 conflicts
        let conflicts = mgr
            .check_file_conflicts(&["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(conflicts.len(), 2);

        let task_ids: Vec<&str> = conflicts.iter().map(|c| c.task_id.as_str()).collect();
        assert!(task_ids.contains(&"task-1"));
        assert!(task_ids.contains(&"task-2"));

        // Only task-2 locks src/util.rs
        let util_conflicts = mgr
            .check_file_conflicts(&["src/util.rs".to_string()])
            .unwrap();
        assert_eq!(util_conflicts.len(), 1);
        assert_eq!(util_conflicts[0].task_id, "task-2");
    }

    #[test]
    fn test_check_conflicts_empty_when_no_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let conflicts = mgr
            .check_file_conflicts(&["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(conflicts.len(), 0);
    }

    #[test]
    fn test_lock_files_persists_active_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files(
            "task-1",
            AgentType::Gemini,
            vec!["a.rs".to_string(), "b.rs".to_string()],
        )
        .unwrap();

        let state = mgr.load().unwrap();
        assert_eq!(state.active_locks.len(), 1);
        let lock = state.active_locks.get("task-1").unwrap();
        assert_eq!(lock.task_id, "task-1");
        assert_eq!(lock.files.len(), 2);
        assert!(lock.files.contains(&"a.rs".to_string()));
        assert!(lock.files.contains(&"b.rs".to_string()));
    }

    // ---------------------------------------------------------------
    // 5. unlock_files
    // ---------------------------------------------------------------

    #[test]
    fn test_unlock_files_removes_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["src/main.rs".to_string()])
            .unwrap();

        // Verify lock exists
        let state = mgr.load().unwrap();
        assert_eq!(state.active_locks.len(), 1);

        // Unlock and verify removal
        mgr.unlock_files("task-1").unwrap();
        let state = mgr.load().unwrap();
        assert!(state.active_locks.is_empty());
    }

    #[test]
    fn test_unlock_files_conflicts_empty_after() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["src/main.rs".to_string()])
            .unwrap();

        // Conflict exists before unlock
        let conflicts = mgr
            .check_file_conflicts(&["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(conflicts.len(), 1);

        // After unlock, no conflicts
        mgr.unlock_files("task-1").unwrap();
        let conflicts = mgr
            .check_file_conflicts(&["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(conflicts.len(), 0);
    }

    #[test]
    fn test_unlock_only_removes_specified_task() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["src/main.rs".to_string()])
            .unwrap();
        mgr.lock_files("task-2", AgentType::Codex, vec!["src/main.rs".to_string()])
            .unwrap();

        mgr.unlock_files("task-1").unwrap();

        let state = mgr.load().unwrap();
        assert_eq!(state.active_locks.len(), 1);
        assert!(state.active_locks.contains_key("task-2"));
        assert!(!state.active_locks.contains_key("task-1"));
    }

    // ---------------------------------------------------------------
    // 6. get_agent_auth — default and explicit values
    // ---------------------------------------------------------------

    #[test]
    fn test_get_agent_auth_default_is_subscription() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let auth = mgr.get_agent_auth("claude").unwrap();
        assert_eq!(auth, "subscription");
    }

    #[test]
    fn test_get_agent_auth_explicit_api_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state
            .agent_auth
            .insert("claude".to_string(), "api".to_string());
        mgr.save(&state).unwrap();

        let auth = mgr.get_agent_auth("claude").unwrap();
        assert_eq!(auth, "api");
    }

    #[test]
    fn test_get_agent_auth_unknown_agent_returns_subscription() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state
            .agent_auth
            .insert("claude".to_string(), "api".to_string());
        mgr.save(&state).unwrap();

        // "gemini" not in map, should fall back to "subscription"
        let auth = mgr.get_agent_auth("gemini").unwrap();
        assert_eq!(auth, "subscription");
    }

    // ---------------------------------------------------------------
    // 7. get_agent_permissions — default and explicit values
    // ---------------------------------------------------------------

    #[test]
    fn test_get_agent_permissions_default_is_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let perms = mgr.get_agent_permissions("claude").unwrap();
        assert_eq!(perms, "safe");
    }

    #[test]
    fn test_get_agent_permissions_explicit_yolo() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state
            .agent_permissions
            .insert("claude".to_string(), "yolo".to_string());
        mgr.save(&state).unwrap();

        let perms = mgr.get_agent_permissions("claude").unwrap();
        assert_eq!(perms, "yolo");
    }

    #[test]
    fn test_get_agent_permissions_unknown_agent_returns_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state
            .agent_permissions
            .insert("claude".to_string(), "yolo".to_string());
        mgr.save(&state).unwrap();

        let perms = mgr.get_agent_permissions("codex").unwrap();
        assert_eq!(perms, "safe");
    }

    // ---------------------------------------------------------------
    // 8. SchedulerConfig::default — assert every field value
    // ---------------------------------------------------------------

    #[test]
    fn test_scheduler_config_default_values() {
        let config = SchedulerConfig::default();
        assert_eq!(config.rotation, false);
        assert_eq!(config.pacing_min_secs, 64);
        assert_eq!(config.pacing_max_secs, 179);
    }

    // ---------------------------------------------------------------
    // 9. refresh_task_summary — count tasks from disk files
    // ---------------------------------------------------------------

    #[test]
    fn test_refresh_task_summary_counts_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        let mgr = StateManager::new(&forge_dir);

        // Save initial state with zeroed summary
        mgr.save(&ForgeState::default()).unwrap();

        // Create task JSON files on disk (TaskManager::list_tasks reads .json)
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let mut t1 = Task::new("T-001", "Pending task", "desc");
        t1.status = TaskStatus::Pending;
        t1.write_json(&forge_dir).unwrap();

        let mut t2 = Task::new("T-002", "Completed task", "desc");
        t2.status = TaskStatus::Completed;
        t2.write_json(&forge_dir).unwrap();

        let mut t3 = Task::new("T-003", "Failed task", "desc");
        t3.status = TaskStatus::Failed;
        t3.write_json(&forge_dir).unwrap();

        let mut t4 = Task::new("T-004", "In-progress task", "desc");
        t4.status = TaskStatus::InProgress;
        t4.write_json(&forge_dir).unwrap();

        let mut t5 = Task::new("T-005", "Assigned task", "desc");
        t5.status = TaskStatus::Assigned;
        t5.write_json(&forge_dir).unwrap();

        let mut t6 = Task::new("T-006", "Blocked task", "desc");
        t6.status = TaskStatus::Blocked;
        t6.write_json(&forge_dir).unwrap();

        mgr.refresh_task_summary().unwrap();
        let loaded = mgr.load().unwrap();

        assert_eq!(loaded.task_summary.total, 6);
        assert_eq!(loaded.task_summary.pending, 1);
        // InProgress + Assigned both count as in_progress
        assert_eq!(loaded.task_summary.in_progress, 2);
        assert_eq!(loaded.task_summary.completed, 1);
        assert_eq!(loaded.task_summary.failed, 1);
        assert_eq!(loaded.task_summary.blocked, 1);
    }

    #[test]
    fn test_refresh_task_summary_empty_tasks_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        let mgr = StateManager::new(&forge_dir);
        mgr.save(&ForgeState::default()).unwrap();

        // No tasks dir at all
        mgr.refresh_task_summary().unwrap();
        let loaded = mgr.load().unwrap();

        assert_eq!(loaded.task_summary.total, 0);
        assert_eq!(loaded.task_summary.pending, 0);
        assert_eq!(loaded.task_summary.in_progress, 0);
        assert_eq!(loaded.task_summary.completed, 0);
        assert_eq!(loaded.task_summary.failed, 0);
        assert_eq!(loaded.task_summary.blocked, 0);
    }

    // ---------------------------------------------------------------
    // 10. update_tools — assert tool details survive roundtrip
    // ---------------------------------------------------------------

    #[test]
    fn test_update_tools_persists_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let tools = vec![
            DetectedTool {
                name: "cargo".to_string(),
                agent_type: AgentType::Any,
                version: Some("1.79.0".to_string()),
                path: "/usr/bin/cargo".to_string(),
                available: true,
            },
            DetectedTool {
                name: "npm".to_string(),
                agent_type: AgentType::Any,
                version: None,
                path: "/usr/bin/npm".to_string(),
                available: false,
            },
        ];
        mgr.update_tools(&tools).unwrap();

        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.tools.len(), 2);
        assert_eq!(loaded.tools[0].name, "cargo");
        assert_eq!(loaded.tools[0].version.as_deref(), Some("1.79.0"));
        assert_eq!(loaded.tools[0].path, "/usr/bin/cargo");
        assert!(loaded.tools[0].available);
        assert_eq!(loaded.tools[1].name, "npm");
        assert!(loaded.tools[1].version.is_none());
        assert_eq!(loaded.tools[1].path, "/usr/bin/npm");
        assert!(!loaded.tools[1].available);
    }

    #[test]
    fn test_update_tools_replaces_previous() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        let tools_v1 = vec![DetectedTool {
            name: "cargo".to_string(),
            agent_type: AgentType::Any,
            version: Some("1.79.0".to_string()),
            path: "/usr/bin/cargo".to_string(),
            available: true,
        }];
        mgr.update_tools(&tools_v1).unwrap();
        assert_eq!(mgr.load().unwrap().tools.len(), 1);

        // Replace with a different set
        let tools_v2 = vec![
            DetectedTool {
                name: "rustc".to_string(),
                agent_type: AgentType::Claude,
                version: Some("1.80.0".to_string()),
                path: "/usr/bin/rustc".to_string(),
                available: true,
            },
            DetectedTool {
                name: "python3".to_string(),
                agent_type: AgentType::Any,
                version: Some("3.12".to_string()),
                path: "/usr/bin/python3".to_string(),
                available: true,
            },
        ];
        mgr.update_tools(&tools_v2).unwrap();
        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.tools.len(), 2);
        assert_eq!(loaded.tools[0].name, "rustc");
        assert_eq!(loaded.tools[1].name, "python3");
    }

    // ---------------------------------------------------------------
    // ForgeState::default — complete field coverage
    // ---------------------------------------------------------------

    #[test]
    fn test_forge_state_default_complete() {
        let state = ForgeState::default();

        assert_eq!(state.version, "1.1.0");
        assert_eq!(state.project_name, "");
        assert!(state.tools.is_empty());
        assert_eq!(state.brain.provider, "rule-based");
        assert!(state.brain.model.is_none());
        assert_eq!(state.task_summary.total, 0);
        assert_eq!(state.task_summary.pending, 0);
        assert_eq!(state.task_summary.in_progress, 0);
        assert_eq!(state.task_summary.completed, 0);
        assert_eq!(state.task_summary.failed, 0);
        assert_eq!(state.task_summary.blocked, 0);
        assert!(state.active_locks.is_empty());
        assert!(state.agent_auth.is_empty());
        assert!(state.agent_permissions.is_empty());
        assert!(state.git.auto_commit);
        assert_eq!(state.git.strategy, "single");
        assert_eq!(state.scheduler.rotation, false);
        assert_eq!(state.scheduler.pacing_min_secs, 64);
        assert_eq!(state.scheduler.pacing_max_secs, 179);
        assert_eq!(state.dashboard_mode, "piped");
        // timestamps should be equal on creation
        assert_eq!(state.created_at, state.updated_at);
    }

    // ---------------------------------------------------------------
    // TaskSummary::default — all zeroes
    // ---------------------------------------------------------------

    #[test]
    fn test_task_summary_default_all_zeroes() {
        let summary = TaskSummary::default();
        assert_eq!(summary.total, 0);
        assert_eq!(summary.pending, 0);
        assert_eq!(summary.in_progress, 0);
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.blocked, 0);
    }

    // ---------------------------------------------------------------
    // Lock replaces itself when same task_id is locked twice
    // ---------------------------------------------------------------

    #[test]
    fn test_lock_same_task_twice_replaces_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["a.rs".to_string()])
            .unwrap();

        // Re-lock with different files
        mgr.lock_files(
            "task-1",
            AgentType::Claude,
            vec!["b.rs".to_string(), "c.rs".to_string()],
        )
        .unwrap();

        let state = mgr.load().unwrap();
        assert_eq!(state.active_locks.len(), 1);
        let lock = state.active_locks.get("task-1").unwrap();
        assert_eq!(lock.files.len(), 2);
        assert!(lock.files.contains(&"b.rs".to_string()));
        assert!(lock.files.contains(&"c.rs".to_string()));
        // Old "a.rs" should not appear
        assert!(!lock.files.contains(&"a.rs".to_string()));
    }

    // ---------------------------------------------------------------
    // Unlock nonexistent task is a no-op (no error, no side effects)
    // ---------------------------------------------------------------

    #[test]
    fn test_unlock_nonexistent_task_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());
        mgr.save(&ForgeState::default()).unwrap();

        mgr.lock_files("task-1", AgentType::Claude, vec!["a.rs".to_string()])
            .unwrap();

        // Unlock a task that does not exist — should not error or affect task-1
        mgr.unlock_files("task-999").unwrap();

        let state = mgr.load().unwrap();
        assert_eq!(state.active_locks.len(), 1);
        assert!(state.active_locks.contains_key("task-1"));
    }

    // ---------------------------------------------------------------
    // Multiple agents in auth and permissions simultaneously
    // ---------------------------------------------------------------

    #[test]
    fn test_multiple_agents_auth_and_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = StateManager::new(tmp.path());

        let mut state = ForgeState::default();
        state
            .agent_auth
            .insert("claude".to_string(), "api".to_string());
        state
            .agent_auth
            .insert("codex".to_string(), "subscription".to_string());
        state
            .agent_permissions
            .insert("claude".to_string(), "yolo".to_string());
        state
            .agent_permissions
            .insert("gemini".to_string(), "safe".to_string());
        mgr.save(&state).unwrap();

        assert_eq!(mgr.get_agent_auth("claude").unwrap(), "api");
        assert_eq!(mgr.get_agent_auth("codex").unwrap(), "subscription");
        assert_eq!(mgr.get_agent_auth("unknown").unwrap(), "subscription");
        assert_eq!(mgr.get_agent_permissions("claude").unwrap(), "yolo");
        assert_eq!(mgr.get_agent_permissions("gemini").unwrap(), "safe");
        assert_eq!(mgr.get_agent_permissions("unknown").unwrap(), "safe");
    }
}
