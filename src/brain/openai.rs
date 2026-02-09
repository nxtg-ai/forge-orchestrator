use super::{DriftScore, ForgeBrain, KnowledgeCategory};
use crate::core::task::{AgentType, Task};
use serde::{Deserialize, Serialize};

/// OpenAI-powered brain — uses GPT-4o, o3, o4-mini, or future models.
///
/// Requires OPENAI_API_KEY environment variable (loaded from .env or shell).
///
/// This brain delegates intelligent decisions to OpenAI's API:
/// - Plan decomposition with structured output
/// - Smart task assignment based on task complexity analysis
/// - Vision drift detection via reasoning
/// - Knowledge classification
///
/// Configuration:
///   forge config set brain openai
///   forge config set brain.model gpt-4.1  # or gpt-5, gpt-5-mini, gpt-5.3-codex
pub struct OpenAIBrain {
    model: String,
    api_key: Option<String>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

impl OpenAIBrain {
    pub fn new(model: impl Into<String>) -> Self {
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        Self {
            model: model.into(),
            api_key,
        }
    }

    pub fn default_model() -> Self {
        Self::new("gpt-4.1")
    }

    fn get_api_key(&self) -> anyhow::Result<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "OPENAI_API_KEY not set. Add it to .env or export it in your shell."
                )
            })
    }

    fn chat(&self, system: &str, user: &str, temperature: f32) -> anyhow::Result<String> {
        let api_key = self.get_api_key()?;

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system.into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: user.into(),
                },
            ],
            temperature,
        };

        let client = reqwest::blocking::Client::new();
        let response = client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&request)
            .send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "OpenAI API error ({}): {}",
                status,
                body
            ));
        }

        let chat_response: ChatResponse = response.json()?;
        let content = chat_response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(content)
    }

    fn has_api_key(&self) -> bool {
        self.api_key.is_some() || std::env::var("OPENAI_API_KEY").is_ok()
    }
}

impl ForgeBrain for OpenAIBrain {
    fn decompose_plan(
        &self,
        spec: &str,
        available_tools: &[AgentType],
    ) -> anyhow::Result<Vec<Task>> {
        if !self.has_api_key() {
            eprintln!("[forge-brain] No API key — falling back to rule-based decomposition");
            let fallback = super::rule_based::RuleBasedBrain;
            return fallback.decompose_plan(spec, available_tools);
        }

        let tools_str = available_tools
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        let system = "You are Forge Orchestrator's planning brain. Given a project specification, \
            decompose it into discrete, actionable tasks. Each task should be a clear unit of work \
            that one AI agent can complete independently.\n\n\
            Output ONLY a JSON array of objects with these fields:\n\
            - \"title\": short task title (3-8 words)\n\
            - \"description\": detailed description of what to implement\n\
            - \"agent\": which AI tool should do this (choose from the available tools)\n\
            - \"depends_on\": array of task indices (0-based) this depends on, or empty array\n\
            - \"acceptance_criteria\": array of testable acceptance criteria strings\n\
            - \"locked_files\": array of file paths this task will modify\n\n\
            Important rules:\n\
            - Assign architecture/design/review tasks to claude\n\
            - Assign implementation/coding tasks to codex (if available, else claude)\n\
            - Assign testing/documentation tasks to gemini (if available, else claude)\n\
            - Keep tasks small enough for one focused session\n\
            - Use dependencies to enforce ordering where needed\n\
            - Output raw JSON only, no markdown fences";

        let user = format!(
            "Available AI tools: {tools_str}\n\nProject specification:\n\n{spec}"
        );

        eprintln!("[forge-brain] Calling OpenAI ({}) for plan decomposition...", self.model);
        let response = self.chat(system, &user, 0.3)?;

        // Parse the JSON response
        let parsed: Vec<serde_json::Value> = serde_json::from_str(response.trim())
            .or_else(|_| {
                // Try extracting JSON from markdown code fences
                let stripped = response
                    .trim()
                    .strip_prefix("```json")
                    .or_else(|| response.trim().strip_prefix("```"))
                    .unwrap_or(response.trim())
                    .strip_suffix("```")
                    .unwrap_or(response.trim());
                serde_json::from_str(stripped)
            })
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse OpenAI response as JSON: {e}\nResponse: {}",
                    &response[..response.len().min(500)]
                )
            })?;

        let mut tasks = Vec::new();
        for (i, item) in parsed.iter().enumerate() {
            let id = format!("T-{:03}", i + 1);
            let title = item["title"].as_str().unwrap_or("Untitled").to_string();
            let description = item["description"]
                .as_str()
                .unwrap_or(&title)
                .to_string();

            let agent_str = item["agent"].as_str().unwrap_or("claude");
            let agent: AgentType = agent_str.parse().unwrap_or(AgentType::Claude);

            let depends_on: Vec<String> = item["depends_on"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|idx| format!("T-{:03}", idx + 1))
                        .collect()
                })
                .unwrap_or_default();

            let acceptance_criteria: Vec<String> = item["acceptance_criteria"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let locked_files: Vec<String> = item["locked_files"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let mut task = Task::new(&id, &title, &description);
            task.assigned_to = Some(agent);
            task.depends_on = depends_on;
            task.acceptance_criteria = acceptance_criteria;
            task.locked_files = locked_files;

            tasks.push(task);
        }

        if tasks.is_empty() {
            eprintln!("[forge-brain] OpenAI returned no tasks — falling back to rule-based");
            let fallback = super::rule_based::RuleBasedBrain;
            return fallback.decompose_plan(spec, available_tools);
        }

        eprintln!("[forge-brain] Generated {} tasks via OpenAI", tasks.len());
        Ok(tasks)
    }

    fn assign_task(
        &self,
        task: &Task,
        available_tools: &[AgentType],
    ) -> anyhow::Result<AgentType> {
        if !self.has_api_key() {
            let fallback = super::rule_based::RuleBasedBrain;
            return fallback.assign_task(task, available_tools);
        }

        let tools_str = available_tools
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        let system = "You are a task assignment brain. Given a task description and available AI tools, \
            pick the best tool. Output ONLY the tool name (one word: claude, codex, or gemini).\n\
            Guidelines:\n\
            - claude: best for architecture, design, review, complex reasoning\n\
            - codex: best for implementation, coding, refactoring, fast code generation\n\
            - gemini: best for testing, documentation, structured output";

        let user = format!(
            "Available tools: {tools_str}\n\nTask: {} — {}\n\nBest tool:",
            task.title, task.description
        );

        let response = self.chat(system, &user, 0.1)?;
        let tool_name = response.trim().to_lowercase();

        tool_name.parse().or_else(|_| {
            // If parsing fails, fall back to rule-based
            let fallback = super::rule_based::RuleBasedBrain;
            fallback.assign_task(task, available_tools)
        })
    }

    fn evaluate_drift(
        &self,
        work_summary: &str,
        vision: &str,
    ) -> anyhow::Result<DriftScore> {
        if !self.has_api_key() {
            return Ok(DriftScore {
                score: 0.0,
                explanation: "No API key — cannot evaluate drift. Set OPENAI_API_KEY.".into(),
            });
        }

        let system = "You are a project governance brain. Evaluate whether completed work aligns \
            with the project vision. Output JSON with two fields:\n\
            - \"score\": float 0.0 to 1.0 (0 = perfectly aligned, 1 = completely off track)\n\
            - \"explanation\": brief explanation of the alignment status\n\n\
            Output raw JSON only, no markdown fences.";

        let user = format!(
            "PROJECT VISION:\n{vision}\n\nCOMPLETED WORK:\n{work_summary}"
        );

        eprintln!("[forge-brain] Calling OpenAI ({}) for drift evaluation...", self.model);
        let response = self.chat(system, &user, 0.2)?;

        // Parse response
        let parsed: serde_json::Value = serde_json::from_str(response.trim())
            .or_else(|_| {
                let stripped = response
                    .trim()
                    .strip_prefix("```json")
                    .or_else(|| response.trim().strip_prefix("```"))
                    .unwrap_or(response.trim())
                    .strip_suffix("```")
                    .unwrap_or(response.trim());
                serde_json::from_str(stripped)
            })
            .unwrap_or_else(|_| {
                serde_json::json!({
                    "score": 0.0,
                    "explanation": format!("Could not parse drift response: {}", &response[..response.len().min(200)])
                })
            });

        Ok(DriftScore {
            score: parsed["score"].as_f64().unwrap_or(0.0) as f32,
            explanation: parsed["explanation"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
        })
    }

    fn route_knowledge(&self, content: &str) -> anyhow::Result<KnowledgeCategory> {
        if !self.has_api_key() {
            let fallback = super::rule_based::RuleBasedBrain;
            return fallback.route_knowledge(content);
        }

        let system = "Classify this content into exactly one category. Output ONLY the category name:\n\
            - research: findings, discoveries, external information\n\
            - decision: choices made, architectural decisions, ADRs\n\
            - learning: lessons learned, mistakes, debugging insights\n\
            - pattern: best practices, conventions, reusable approaches";

        let response = self.chat(system, content, 0.1)?;
        let category = response.trim().to_lowercase();

        Ok(match category.as_str() {
            "research" => KnowledgeCategory::Research,
            "decision" | "decisions" => KnowledgeCategory::Decision,
            "learning" | "learnings" => KnowledgeCategory::Learning,
            "pattern" | "patterns" => KnowledgeCategory::Pattern,
            _ => {
                // Fall back to rule-based if LLM gives unexpected output
                let fallback = super::rule_based::RuleBasedBrain;
                fallback.route_knowledge(content)?
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_brain_fallback_without_key() {
        // Without API key, should fall back to rule-based
        let brain = OpenAIBrain {
            model: "gpt-4o".into(),
            api_key: None,
        };
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

    #[test]
    fn test_drift_without_key() {
        let brain = OpenAIBrain {
            model: "gpt-4o".into(),
            api_key: None,
        };
        let drift = brain.evaluate_drift("some work", "some vision").unwrap();
        assert_eq!(drift.score, 0.0);
        assert!(drift.explanation.contains("API key"));
    }

    #[test]
    fn test_knowledge_routing_without_key() {
        let brain = OpenAIBrain {
            model: "gpt-4o".into(),
            api_key: None,
        };
        // Should fall back to rule-based
        let cat = brain
            .route_knowledge("Research finding: Rust is fast")
            .unwrap();
        assert_eq!(cat, KnowledgeCategory::Research);
    }
}
