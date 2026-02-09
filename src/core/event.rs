use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

use super::task::AgentType;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    PlanCreated,
    TaskCreated,
    TaskAssigned,
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    FilesLocked,
    FilesUnlocked,
    KnowledgeCaptured,
    GovernanceCheck,
    StateReconciled,
    ToolDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub task_id: Option<String>,
    pub agent: Option<AgentType>,
    pub message: String,
    pub metadata: Option<serde_json::Value>,
}

impl ForgeEvent {
    pub fn new(event_type: EventType, message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            task_id: None,
            agent: None,
            message: message.into(),
            metadata: None,
        }
    }

    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_agent(mut self, agent: AgentType) -> Self {
        self.agent = Some(agent);
        self
    }

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
