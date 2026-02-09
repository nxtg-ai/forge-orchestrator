use super::{DriftScore, ForgeBrain, KnowledgeCategory};
use crate::core::task::{AgentType, Task};

/// OpenAI-powered brain — uses GPT-4o, o3, o4-mini, or future models.
///
/// Requires OPENAI_API_KEY environment variable.
///
/// This brain delegates intelligent decisions to OpenAI's API:
/// - Plan decomposition with structured output (JSON schema)
/// - Smart task assignment based on task complexity analysis
/// - Vision drift detection via reasoning models (o3)
/// - Knowledge classification via fast models (o4-mini)
///
/// Configuration:
///   forge config set brain openai
///   forge config set brain.model gpt-4o  # or o3, o4-mini
pub struct OpenAIBrain {
    model: String,
}

impl OpenAIBrain {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }

    pub fn default_model() -> Self {
        Self {
            model: "gpt-4o".into(),
        }
    }

    fn _get_api_key() -> anyhow::Result<String> {
        std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set. Set it to use the OpenAI brain."))
    }
}

impl ForgeBrain for OpenAIBrain {
    fn decompose_plan(
        &self,
        spec: &str,
        available_tools: &[AgentType],
    ) -> anyhow::Result<Vec<Task>> {
        // TODO: Phase 5 — Call OpenAI API with structured output
        // For now, fall back to rule-based decomposition
        eprintln!(
            "OpenAI brain (model: {}) not yet connected. Using rule-based fallback.",
            self.model
        );
        let fallback = super::rule_based::RuleBasedBrain;
        fallback.decompose_plan(spec, available_tools)
    }

    fn assign_task(
        &self,
        task: &Task,
        available_tools: &[AgentType],
    ) -> anyhow::Result<AgentType> {
        // TODO: Phase 5 — Ask OpenAI to analyze task and pick best tool
        let fallback = super::rule_based::RuleBasedBrain;
        fallback.assign_task(task, available_tools)
    }

    fn evaluate_drift(
        &self,
        _work_summary: &str,
        _vision: &str,
    ) -> anyhow::Result<DriftScore> {
        // TODO: Phase 5 — Use o3 reasoning model for drift analysis
        Ok(DriftScore {
            score: 0.0,
            explanation: format!(
                "OpenAI drift evaluation (model: {}) not yet implemented.",
                self.model
            ),
        })
    }

    fn route_knowledge(&self, content: &str) -> anyhow::Result<KnowledgeCategory> {
        // TODO: Phase 5 — Use o4-mini for fast classification
        let fallback = super::rule_based::RuleBasedBrain;
        fallback.route_knowledge(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_brain_fallback() {
        let brain = OpenAIBrain::default_model();
        let spec = "## Auth\n## Database\n";
        let tasks = brain
            .decompose_plan(spec, &[AgentType::Claude])
            .unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_openai_brain_model_config() {
        let brain = OpenAIBrain::new("o3");
        assert_eq!(brain.model, "o3");
    }
}
