use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

use super::task::AgentType;

/// What an event records. Serialized in snake_case (`task_completed`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// A master plan was written.
    PlanCreated,
    /// A task was created.
    TaskCreated,
    /// A task was assigned to an agent.
    TaskAssigned,
    /// An agent started a task.
    TaskStarted,
    /// A task completed.
    TaskCompleted,
    /// A task failed.
    TaskFailed,
    /// Files were locked for a task.
    FilesLocked,
    /// A task's file locks were released.
    FilesUnlocked,
    /// A knowledge entry was captured.
    KnowledgeCaptured,
    /// A governance health check ran.
    GovernanceCheck,
    /// `forge sync` reconciled the state file.
    StateReconciled,
    /// An AI tool was detected on PATH.
    ToolDetected,
    /// A quality gate passed.
    QualityGatePassed,
    /// A quality gate failed.
    QualityGateFailed,
    /// A pod pane died and was respawned by `forge pod` recovery, re-claiming its bound task.
    ///
    /// Adding a variant is a MINOR schema change per RFC-0001 §3.1 — safe only because readers
    /// map unknown variants to `Unknown` below instead of erroring the whole batch.
    PaneRecovered,
    /// Forward-compatibility sink: a variant written by a newer forge.
    ///
    /// RFC-0001 §3.2 rule 3. Without this, the first record carrying a future variant turns an
    /// older consumer's event-log read from "returns 500 events" into "errors on the batch".
    #[serde(other)]
    Unknown,
}

/// One line of `.forge/events.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEvent {
    /// When the event was recorded (UTC).
    pub timestamp: DateTime<Utc>,
    /// What happened.
    pub event_type: EventType,
    /// The task it concerns, if any.
    pub task_id: Option<String>,
    /// The agent it concerns, if any.
    pub agent: Option<AgentType>,
    /// Human-readable description.
    pub message: String,
    /// Task duration in milliseconds (from dispatch to completion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Process exit code (0 = success).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Free-form structured data.
    pub metadata: Option<serde_json::Value>,
}

impl ForgeEvent {
    /// A new event stamped with the current time and no task, agent or extras.
    pub fn new(event_type: EventType, message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            task_id: None,
            agent: None,
            message: message.into(),
            duration_ms: None,
            exit_code: None,
            metadata: None,
        }
    }

    /// Sets the task id.
    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Sets the agent.
    pub fn with_agent(mut self, agent: AgentType) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Sets the task duration in milliseconds.
    pub fn with_duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    /// Sets the process exit code.
    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }

    /// Sets the structured metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Append-only event log at .forge/events.jsonl
pub struct EventLogger {
    forge_dir: PathBuf,
}

impl EventLogger {
    /// A logger for the given `.forge/` directory. Nothing is read or written until used.
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    fn events_path(&self) -> PathBuf {
        self.forge_dir.join("events.jsonl")
    }

    /// Append a single event to the JSONL log
    pub fn log(&self, event: &ForgeEvent) -> anyhow::Result<()> {
        tracing::debug!(
            event_type = ?event.event_type,
            task_id = ?event.task_id,
            agent = ?event.agent,
            "Writing event"
        );
        std::fs::create_dir_all(&self.forge_dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;

        let line = serde_json::to_string(event)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Read all events (for status display)
    pub fn read_all(&self) -> anyhow::Result<Vec<ForgeEvent>> {
        let path = self.events_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&path)?;
        let events: Vec<ForgeEvent> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(events)
    }

    /// Read the last N events
    pub fn read_recent(&self, count: usize) -> anyhow::Result<Vec<ForgeEvent>> {
        let all = self.read_all()?;
        let start = all.len().saturating_sub(count);
        Ok(all[start..].to_vec())
    }
}
