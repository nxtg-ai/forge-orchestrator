use super::{DriftScore, ForgeBrain, KnowledgeCategory};
use crate::core::task::{AgentType, Task};

/// Rule-based brain — no LLM calls, pure heuristics.
/// This is the free tier / fallback brain.
pub struct RuleBasedBrain;

/// Classify a task's type from its title and description using keyword heuristics.
pub fn classify_task_type(title: &str, description: &str) -> Option<String> {
    let text = format!("{} {}", title.to_lowercase(), description.to_lowercase());

    if text.contains("design")
        || text.contains("architect")
        || text.contains("plan")
        || text.contains("schema")
        || text.contains("strategy")
    {
        return Some("design".into());
    }

    if text.contains("review")
        || text.contains("audit")
        || text.contains("analyze")
        || text.contains("evaluate")
        || text.contains("inspect")
    {
        return Some("review".into());
    }

    if text.contains("test")
        || text.contains("spec")
        || text.contains("coverage")
        || text.contains("assert")
    {
        return Some("test".into());
    }

    if text.contains("document")
        || text.contains("readme")
        || text.contains("docs")
        || text.contains("api doc")
        || text.contains("comment")
    {
        return Some("document".into());
    }

    if text.contains("implement")
        || text.contains("build")
        || text.contains("create")
        || text.contains("add")
        || text.contains("fix")
        || text.contains("refactor")
        || text.contains("code")
        || text.contains("develop")
    {
        return Some("implement".into());
    }

    None
}

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
                let title = header.trim();
                let desc = format!("Implement: {title}");
                let mut task = Task::new(&id, title, &desc);
                task.task_type = classify_task_type(title, &desc);
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

    fn assign_task(&self, task: &Task, available_tools: &[AgentType]) -> anyhow::Result<AgentType> {
        if available_tools.is_empty() {
            return Ok(AgentType::Any);
        }

        let title_lower = task.title.to_lowercase();
        let desc_lower = task.description.to_lowercase();
        let text = format!("{title_lower} {desc_lower}");

        // Keyword-based assignment heuristics (check title + description)
        if text.contains("architect")
            || text.contains("design")
            || text.contains("plan")
            || text.contains("review")
            || text.contains("evaluate")
            || text.contains("decide")
            || text.contains("strategy")
        {
            // Architecture/strategy tasks → Claude (strongest reasoning)
            if available_tools.contains(&AgentType::Claude) {
                return Ok(AgentType::Claude);
            }
        }

        if text.contains("implement")
            || text.contains("code")
            || text.contains("build")
            || text.contains("refactor")
            || text.contains("create")
            || text.contains("add")
            || text.contains("fix")
            || text.contains("develop")
            || text.contains("write code")
        {
            // Implementation tasks → Codex (fast code generation)
            if available_tools.contains(&AgentType::Codex) {
                return Ok(AgentType::Codex);
            }
        }

        if text.contains("test")
            || text.contains("doc")
            || text.contains("document")
            || text.contains("documentation")
            || text.contains("report")
            || text.contains("summary")
            || text.contains("validate")
            || text.contains("verify")
            || text.contains("quality")
            || text.contains("coverage")
        {
            // Testing/docs/validation → Gemini (good at structured output)
            if available_tools.contains(&AgentType::Gemini) {
                return Ok(AgentType::Gemini);
            }
        }

        // Round-robin fallback: distribute by task ID number
        let task_num: usize = task.id.trim_start_matches("T-").parse().unwrap_or(0);
        let idx = task_num % available_tools.len();
        Ok(available_tools[idx].clone())
    }

    fn evaluate_drift(&self, _work_summary: &str, _vision: &str) -> anyhow::Result<DriftScore> {
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
    fn test_assign_testing_to_gemini() {
        let brain = RuleBasedBrain;
        let task = Task::new("T-003", "Write unit tests for auth module", "...");
        let tools = vec![AgentType::Claude, AgentType::Codex, AgentType::Gemini];
        let result = brain.assign_task(&task, &tools).unwrap();
        assert_eq!(result, AgentType::Gemini);
    }

    #[test]
    fn test_assign_docs_to_gemini() {
        let brain = RuleBasedBrain;
        let task = Task::new("T-004", "API documentation update", "Update the API docs");
        let tools = vec![AgentType::Claude, AgentType::Codex, AgentType::Gemini];
        let result = brain.assign_task(&task, &tools).unwrap();
        assert_eq!(result, AgentType::Gemini);
    }

    #[test]
    fn test_fallback_round_robin() {
        let brain = RuleBasedBrain;
        // "Setup" doesn't match any keyword heuristic — falls through to round-robin
        let task1 = Task::new("T-003", "Setup environment", "Configure dev setup");
        let task2 = Task::new("T-004", "Setup environment", "Configure dev setup");
        let tools = vec![AgentType::Claude, AgentType::Codex, AgentType::Gemini];
        let r1 = brain.assign_task(&task1, &tools).unwrap(); // 3 % 3 = 0 → Claude
        let r2 = brain.assign_task(&task2, &tools).unwrap(); // 4 % 3 = 1 → Codex
        assert_eq!(r1, AgentType::Claude);
        assert_eq!(r2, AgentType::Codex);
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
