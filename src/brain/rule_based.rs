use super::{DriftScore, ForgeBrain, KnowledgeCategory};
use crate::core::task::{AgentType, Task};

/// Rule-based brain — no LLM calls, pure heuristics.
/// This is the free tier / fallback brain.
pub struct RuleBasedBrain;

impl ForgeBrain for RuleBasedBrain {
    fn decompose_plan(
        &self,
        spec: &str,
        _available_tools: &[AgentType],
    ) -> anyhow::Result<Vec<Task>> {
        // Simple heuristic: split spec by markdown headers into tasks
        let mut tasks = Vec::new();
        let mut task_num = 1;

        for line in spec.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let id = format!("T-{task_num:03}");
                let task = Task::new(&id, header.trim(), format!("Implement: {}", header.trim()));
                tasks.push(task);
                task_num += 1;
            }
        }

        // If no headers found, create a single task
        if tasks.is_empty() {
            tasks.push(Task::new(
                "T-001",
                "Implement specification",
                spec.chars().take(500).collect::<String>(),
            ));
        }

        Ok(tasks)
    }

    fn assign_task(
        &self,
        task: &Task,
        available_tools: &[AgentType],
    ) -> anyhow::Result<AgentType> {
        if available_tools.is_empty() {
            return Ok(AgentType::Any);
        }

        let title_lower = task.title.to_lowercase();

        // Simple keyword-based assignment heuristics
        if title_lower.contains("architect")
            || title_lower.contains("design")
            || title_lower.contains("plan")
            || title_lower.contains("review")
        {
            // Architecture tasks → Claude (strongest reasoning)
            if available_tools.contains(&AgentType::Claude) {
                return Ok(AgentType::Claude);
            }
        }

        if title_lower.contains("implement")
            || title_lower.contains("code")
            || title_lower.contains("build")
            || title_lower.contains("refactor")
        {
            // Implementation tasks → Codex (fast code generation)
            if available_tools.contains(&AgentType::Codex) {
                return Ok(AgentType::Codex);
            }
        }

        if title_lower.contains("test")
            || title_lower.contains("doc")
            || title_lower.contains("document")
        {
            // Testing/docs → Gemini (good at structured output)
            if available_tools.contains(&AgentType::Gemini) {
                return Ok(AgentType::Gemini);
            }
        }

        // Default: use the first available tool
        Ok(available_tools[0].clone())
    }

    fn evaluate_drift(
        &self,
        _work_summary: &str,
        _vision: &str,
    ) -> anyhow::Result<DriftScore> {
        // Rule-based brain can't evaluate drift without LLM
        // Return a neutral score
        Ok(DriftScore {
            score: 0.0,
            explanation: "Drift evaluation requires an LLM brain. Set brain to claude or gemini for intelligent drift detection.".into(),
        })
    }

    fn route_knowledge(&self, content: &str) -> anyhow::Result<KnowledgeCategory> {
        let lower = content.to_lowercase();

        // Keyword-based classification
        if lower.contains("research")
            || lower.contains("finding")
            || lower.contains("discovered")
            || lower.contains("learned that")
        {
            return Ok(KnowledgeCategory::Research);
        }

        if lower.contains("decided")
            || lower.contains("decision")
            || lower.contains("chose")
            || lower.contains("adr")
        {
            return Ok(KnowledgeCategory::Decision);
        }

        if lower.contains("lesson")
            || lower.contains("mistake")
            || lower.contains("bug")
            || lower.contains("fix")
        {
            return Ok(KnowledgeCategory::Learning);
        }

        if lower.contains("pattern")
            || lower.contains("convention")
            || lower.contains("best practice")
        {
            return Ok(KnowledgeCategory::Pattern);
        }

        Ok(KnowledgeCategory::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_architecture_to_claude() {
        let brain = RuleBasedBrain;
        let task = Task::new("T-001", "Design the API architecture", "...");
        let tools = vec![AgentType::Claude, AgentType::Codex];
        let result = brain.assign_task(&task, &tools).unwrap();
        assert_eq!(result, AgentType::Claude);
    }

    #[test]
    fn test_assign_implementation_to_codex() {
        let brain = RuleBasedBrain;
        let task = Task::new("T-002", "Implement the login endpoint", "...");
        let tools = vec![AgentType::Claude, AgentType::Codex];
        let result = brain.assign_task(&task, &tools).unwrap();
        assert_eq!(result, AgentType::Codex);
    }

    #[test]
    fn test_route_knowledge_research() {
        let brain = RuleBasedBrain;
        let result = brain
            .route_knowledge("Research finding: Codex CLI supports MCP server mode")
            .unwrap();
        assert_eq!(result, KnowledgeCategory::Research);
    }

    #[test]
    fn test_route_knowledge_decision() {
        let brain = RuleBasedBrain;
        let result = brain
            .route_knowledge("We decided to use Rust for the orchestrator")
            .unwrap();
        assert_eq!(result, KnowledgeCategory::Decision);
    }

    #[test]
    fn test_decompose_plan_from_headers() {
        let brain = RuleBasedBrain;
        let spec = "# My Project\n## Authentication\n## Database\n## API endpoints\n";
        let tasks = brain.decompose_plan(spec, &[AgentType::Claude]).unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "Authentication");
        assert_eq!(tasks[1].title, "Database");
    }
}
