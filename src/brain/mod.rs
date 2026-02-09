pub mod rule_based;

use crate::core::task::{AgentType, Task};

/// The pluggable "brain" trait for Forge's intelligent decisions.
///
/// Brain 1 (Deterministic Engine) handles 80% of work: state management,
/// file locking, event logging, git ops — all in Rust, zero tokens.
///
/// Brain 2 (ForgeBrain) handles the 20% that requires reasoning:
/// plan decomposition, task assignment, drift detection, knowledge routing.
///
/// Implementations:
/// - RuleBasedBrain: heuristics only, no LLM (free tier)
/// - ClaudeOpusBrain: Claude API (future)
/// - GeminiBrain: Gemini API (future)
/// - LocalBrain: fine-tuned local model (future)
pub trait ForgeBrain {
    /// Decompose a specification into tasks
    fn decompose_plan(&self, spec: &str, available_tools: &[AgentType]) -> anyhow::Result<Vec<Task>>;

    /// Decide which agent should handle a task
    fn assign_task(&self, task: &Task, available_tools: &[AgentType]) -> anyhow::Result<AgentType>;

    /// Evaluate if work is drifting from the vision
    fn evaluate_drift(&self, work_summary: &str, vision: &str) -> anyhow::Result<DriftScore>;

    /// Classify content for knowledge routing
    fn route_knowledge(&self, content: &str) -> anyhow::Result<KnowledgeCategory>;
}

#[derive(Debug, Clone)]
pub struct DriftScore {
    pub score: f32, // 0.0 = perfect alignment, 1.0 = total drift
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KnowledgeCategory {
    Research,
    Decision,
    Learning,
    Pattern,
    Unknown,
}

impl KnowledgeCategory {
    pub fn directory_name(&self) -> &str {
        match self {
            KnowledgeCategory::Research => "research",
            KnowledgeCategory::Decision => "decisions",
            KnowledgeCategory::Learning => "learnings",
            KnowledgeCategory::Pattern => "patterns",
            KnowledgeCategory::Unknown => "uncategorized",
        }
    }
}
