use crate::brain::ForgeBrain;
use crate::brain::rule_based::RuleBasedBrain;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::governance::GovernanceChecker;
use crate::core::knowledge::{KnowledgeEntry, KnowledgeManager};
use crate::core::plan::PlanManager;
use crate::core::state::StateManager;
use crate::core::task::{AgentType, TaskManager, TaskPhase, TaskStatus};
use serde_json::{Value, json};
use std::path::Path;

use super::protocol::{CallToolResult, ToolDefinition};

/// Return all tool definitions for tools/list
pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "forge_get_tasks".into(),
            description: "List all tasks with their current status, assignments, and dependencies"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Filter by status: pending, assigned, in_progress, completed, failed, blocked",
                        "enum": ["pending", "assigned", "in_progress", "completed", "failed", "blocked"]
                    }
                }
            }),
        },
        ToolDefinition {
            name: "forge_claim_task".into(),
            description:
                "Claim a task for an agent — locks associated files and sets status to assigned"
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (e.g., T-001)"
                    },
                    "agent": {
                        "type": "string",
                        "description": "Agent claiming the task: claude, codex, or gemini",
                        "enum": ["claude", "codex", "gemini"]
                    }
                },
                "required": ["task_id", "agent"]
            }),
        },
        ToolDefinition {
            name: "forge_complete_task".into(),
            description: "Mark a task as completed — unlocks files, logs event, updates state"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (e.g., T-001)"
                    },
                    "result_summary": {
                        "type": "string",
                        "description": "Brief summary of what was accomplished"
                    }
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "forge_get_state".into(),
            description: "Get the full orchestration state — project info, tools, task summary, active locks".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "forge_get_plan".into(),
            description: "Read the master plan (plan.md) — shows task board and details".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "forge_capture_knowledge".into(),
            description: "Capture a piece of knowledge — auto-classifies into research, decision, learning, or pattern".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short title for this knowledge entry"
                    },
                    "content": {
                        "type": "string",
                        "description": "The knowledge content to capture"
                    },
                    "category": {
                        "type": "string",
                        "description": "Optional category override (auto-classified if omitted)",
                        "enum": ["research", "decisions", "learnings", "patterns"]
                    },
                    "source": {
                        "type": "string",
                        "description": "Where this knowledge came from (e.g., 'code review', 'debugging session')"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Related task ID (e.g., T-001)"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tags for easier search"
                    }
                },
                "required": ["title", "content"]
            }),
        },
        ToolDefinition {
            name: "forge_get_knowledge".into(),
            description: "Query the knowledge base — list by category or search by keyword".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Filter by category",
                        "enum": ["research", "decisions", "learnings", "patterns"]
                    },
                    "query": {
                        "type": "string",
                        "description": "Search keyword (searches titles, content, and tags)"
                    },
                    "generate_skills": {
                        "type": "boolean",
                        "description": "If true, also generate SKILL.md files from knowledge"
                    }
                }
            }),
        },
        ToolDefinition {
            name: "forge_check_drift".into(),
            description: "Check if work is drifting from the project vision — compares completed tasks against SPEC.md".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "forge_get_health".into(),
            description: "Run a comprehensive governance health check — documentation, architecture, task health, knowledge coverage, and drift detection".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "forge_get_events".into(),
            description: "Query the event log — returns recent events with optional filters".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "count": {
                        "type": "integer",
                        "description": "Number of events to return (default: 50)",
                        "default": 50
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Filter events by task ID (e.g., T-001)"
                    },
                    "event_type": {
                        "type": "string",
                        "description": "Filter by event type",
                        "enum": ["plan_created", "task_created", "task_assigned", "task_started",
                                 "task_completed", "task_failed", "files_locked", "files_unlocked",
                                 "knowledge_captured", "governance_check", "state_reconciled",
                                 "tool_detected", "quality_gate_passed", "quality_gate_failed"]
                    }
                }
            }),
        },
        ToolDefinition {
            name: "forge_set_project".into(),
            description: "Switch the active project — changes which .forge/ directory is used for all subsequent tool calls".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the project root (must contain a .forge/ directory)"
                    }
                },
                "required": ["path"]
            }),
        },
    ]
}

/// Handle forge_set_project — returns Ok(new_path) or Err(error_result)
pub fn handle_set_project(args: &Value) -> Result<std::path::PathBuf, CallToolResult> {
    let path_str = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return Err(CallToolResult::error("Missing required parameter: path")),
    };

    let new_root = std::path::PathBuf::from(path_str);
    if !new_root.join(".forge").exists() {
        return Err(CallToolResult::error(format!(
            "No .forge/ directory found at {path_str}. Run `forge init` in that project first."
        )));
    }

    Ok(new_root)
}

/// Dispatch a tool call to the appropriate handler
pub fn call_tool(name: &str, args: &Value, project_root: &Path) -> CallToolResult {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        return CallToolResult::error(
            "Forge is not initialized. Run `forge init` in this project first.",
        );
    }

    tracing::debug!(tool = name, "MCP tool call");
    match name {
        "forge_get_tasks" => handle_get_tasks(args, &forge_dir),
        "forge_claim_task" => handle_claim_task(args, &forge_dir),
        "forge_complete_task" => handle_complete_task(args, &forge_dir),
        "forge_get_state" => handle_get_state(&forge_dir),
        "forge_get_plan" => handle_get_plan(&forge_dir),
        "forge_capture_knowledge" => handle_capture_knowledge(args, &forge_dir),
        "forge_get_knowledge" => handle_get_knowledge(args, &forge_dir),
        "forge_check_drift" => handle_check_drift(project_root),
        "forge_get_health" => handle_get_health(project_root),
        "forge_get_events" => handle_get_events(args, &forge_dir),
        _ => CallToolResult::error(format!("Unknown tool: {name}")),
    }
}

fn handle_get_tasks(args: &Value, forge_dir: &Path) -> CallToolResult {
    let task_mgr = TaskManager::new(forge_dir);

    let tasks = match task_mgr.list_tasks() {
        Ok(t) => t,
        Err(e) => return CallToolResult::error(format!("Failed to list tasks: {e}")),
    };

    // Optional status filter
    let status_filter = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let filtered: Vec<_> = if let Some(ref filter) = status_filter {
        tasks
            .into_iter()
            .filter(|t| {
                let status_str = serde_json::to_value(&t.status)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                status_str == *filter
            })
            .collect()
    } else {
        tasks
    };

    let result = json!({
        "count": filtered.len(),
        "tasks": filtered.iter().map(|t| json!({
            "id": t.id,
            "title": t.title,
            "description": t.description,
            "status": t.status,
            "assigned_to": t.assigned_to,
            "depends_on": t.depends_on,
            "locked_files": t.locked_files,
            "acceptance_criteria": t.acceptance_criteria,
        })).collect::<Vec<_>>()
    });

    CallToolResult::text(serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn handle_claim_task(args: &Value, forge_dir: &Path) -> CallToolResult {
    let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return CallToolResult::error("Missing required parameter: task_id"),
    };

    let agent_str = match args.get("agent").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => return CallToolResult::error("Missing required parameter: agent"),
    };

    let agent: AgentType = match agent_str.parse() {
        Ok(a) => a,
        Err(e) => return CallToolResult::error(format!("Invalid agent: {e}")),
    };

    let task_mgr = TaskManager::new(forge_dir);
    let state_mgr = StateManager::new(forge_dir);
    let event_logger = EventLogger::new(forge_dir);

    // Load task
    let mut task = match task_mgr.get_task(task_id) {
        Ok(t) => t,
        Err(e) => return CallToolResult::error(format!("Task not found: {e}")),
    };

    // UAT tasks are for human validation only
    if task.phase == Some(TaskPhase::Uat) {
        return CallToolResult::error("UAT tasks are for human validation only");
    }

    // Check if already claimed
    if task.status != TaskStatus::Pending {
        return CallToolResult::error(format!(
            "Task {} is not pending (current status: {:?})",
            task_id, task.status
        ));
    }

    // Check dependencies
    let completed = task_mgr.get_completed_task_ids().unwrap_or_default();
    if task.is_blocked(&completed) {
        return CallToolResult::error(format!(
            "Task {} is blocked by: {}",
            task_id,
            task.depends_on.join(", ")
        ));
    }

    // Check file conflicts
    if !task.locked_files.is_empty() {
        let conflicts = state_mgr
            .check_file_conflicts(&task.locked_files)
            .unwrap_or_default();
        if !conflicts.is_empty() {
            let conflict_info: Vec<String> = conflicts
                .iter()
                .map(|c| format!("{} (locked by {})", c.task_id, c.agent))
                .collect();
            return CallToolResult::error(format!(
                "File conflicts detected: {}",
                conflict_info.join(", ")
            ));
        }
    }

    // Claim the task
    task.status = TaskStatus::Assigned;
    task.assigned_to = Some(agent.clone());
    task.updated_at = chrono::Utc::now();

    if let Err(e) = task_mgr.update_task(&task) {
        return CallToolResult::error(format!("Failed to update task: {e}"));
    }

    // Lock files
    if !task.locked_files.is_empty()
        && let Err(e) = state_mgr.lock_files(task_id, agent.clone(), task.locked_files.clone())
    {
        return CallToolResult::error(format!("Failed to lock files: {e}"));
    }

    // Log event
    if let Err(e) = event_logger.log(
        &ForgeEvent::new(
            EventType::TaskAssigned,
            format!("Task {task_id} claimed by {agent}"),
        )
        .with_task(task_id)
        .with_agent(agent.clone()),
    ) {
        eprintln!("[forge-orchestrator] event log error: {e}");
    }

    // Refresh task summary so state.json stays current
    if let Err(e) = state_mgr.refresh_task_summary() {
        eprintln!("[forge-orchestrator] state summary refresh error: {e}");
    }

    CallToolResult::text(format!(
        "Task {task_id} claimed by {agent}. Files locked: {}",
        if task.locked_files.is_empty() {
            "none".to_string()
        } else {
            task.locked_files.join(", ")
        }
    ))
}

fn handle_complete_task(args: &Value, forge_dir: &Path) -> CallToolResult {
    let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return CallToolResult::error("Missing required parameter: task_id"),
    };

    let result_summary = args
        .get("result_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("Task completed");

    let task_mgr = TaskManager::new(forge_dir);
    let state_mgr = StateManager::new(forge_dir);
    let event_logger = EventLogger::new(forge_dir);

    // Load task
    let mut task = match task_mgr.get_task(task_id) {
        Ok(t) => t,
        Err(e) => return CallToolResult::error(format!("Task not found: {e}")),
    };

    // Mark completed
    let now = chrono::Utc::now();
    task.status = TaskStatus::Completed;
    task.updated_at = now;
    task.completed_at = Some(now);

    if let Err(e) = task_mgr.update_task(&task) {
        return CallToolResult::error(format!("Failed to update task: {e}"));
    }

    // Unlock files
    if let Err(e) = state_mgr.unlock_files(task_id) {
        eprintln!("[forge-orchestrator] file unlock error for {task_id}: {e}");
    }

    // Log event
    if let Err(e) = event_logger.log(
        &ForgeEvent::new(
            EventType::TaskCompleted,
            format!("Task {task_id} completed: {result_summary}"),
        )
        .with_task(task_id),
    ) {
        eprintln!("[forge-orchestrator] event log error: {e}");
    }

    // Refresh task summary so state.json stays current
    if let Err(e) = state_mgr.refresh_task_summary() {
        eprintln!("[forge-orchestrator] state summary refresh error: {e}");
    }

    // Check for newly unblocked tasks
    let completed_ids = task_mgr.get_completed_task_ids().unwrap_or_default();
    let all_tasks = task_mgr.list_tasks().unwrap_or_default();
    let newly_available: Vec<String> = all_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending && !t.is_blocked(&completed_ids))
        .map(|t| format!("{}: {}", t.id, t.title))
        .collect();

    let mut response = format!("Task {task_id} completed.");
    if !newly_available.is_empty() {
        response.push_str(&format!(
            "\n\nNewly available tasks:\n{}",
            newly_available
                .iter()
                .map(|t| format!("  - {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    CallToolResult::text(response)
}

fn handle_get_state(forge_dir: &Path) -> CallToolResult {
    let state_mgr = StateManager::new(forge_dir);

    match state_mgr.load() {
        Ok(state) => {
            let json = serde_json::to_string_pretty(&state).unwrap_or_default();
            CallToolResult::text(json)
        }
        Err(e) => CallToolResult::error(format!("Failed to load state: {e}")),
    }
}

fn handle_get_plan(forge_dir: &Path) -> CallToolResult {
    let plan_mgr = PlanManager::new(forge_dir);

    if plan_mgr.has_plan() {
        match plan_mgr.read_plan() {
            Ok(content) => CallToolResult::text(content),
            Err(e) => CallToolResult::error(format!("Failed to read plan: {e}")),
        }
    } else {
        CallToolResult::text(
            "No plan exists yet. Use `forge plan --generate` to create one from SPEC.md.",
        )
    }
}

fn handle_capture_knowledge(args: &Value, forge_dir: &Path) -> CallToolResult {
    let title = match args.get("title").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return CallToolResult::error("Missing required parameter: title"),
    };

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return CallToolResult::error("Missing required parameter: content"),
    };

    // Classify: use explicit category if provided, otherwise auto-classify
    let category = if let Some(cat_str) = args.get("category").and_then(|v| v.as_str()) {
        match cat_str {
            "research" => crate::brain::KnowledgeCategory::Research,
            "decisions" => crate::brain::KnowledgeCategory::Decision,
            "learnings" => crate::brain::KnowledgeCategory::Learning,
            "patterns" => crate::brain::KnowledgeCategory::Pattern,
            _ => crate::brain::KnowledgeCategory::Unknown,
        }
    } else {
        // Auto-classify using the brain
        let brain = RuleBasedBrain;
        brain
            .route_knowledge(content)
            .unwrap_or(crate::brain::KnowledgeCategory::Unknown)
    };

    let knowledge_mgr = KnowledgeManager::new(forge_dir);
    let event_logger = EventLogger::new(forge_dir);

    let mut entry = KnowledgeEntry::new(title, content, &category);

    if let Some(source) = args.get("source").and_then(|v| v.as_str()) {
        entry = entry.with_source(source);
    }
    if let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) {
        entry = entry.with_task(task_id);
    }
    if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
        let tag_strings: Vec<String> = tags
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
        entry = entry.with_tags(tag_strings);
    }

    let entry_id = entry.id.clone();
    let cat_name = entry.category.clone();

    if let Err(e) = knowledge_mgr.capture(&entry) {
        return CallToolResult::error(format!("Failed to capture knowledge: {e}"));
    }

    // Log event
    if let Err(e) = event_logger.log(&ForgeEvent::new(
        EventType::KnowledgeCaptured,
        format!("Knowledge captured: {title} (category: {cat_name})"),
    )) {
        eprintln!("[forge-orchestrator] event log error: {e}");
    }

    CallToolResult::text(format!(
        "Knowledge captured:\n  ID: {entry_id}\n  Category: {cat_name}\n  Title: {title}"
    ))
}

fn handle_get_knowledge(args: &Value, forge_dir: &Path) -> CallToolResult {
    let knowledge_mgr = KnowledgeManager::new(forge_dir);

    // Search mode
    if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
        let results = match knowledge_mgr.search(query) {
            Ok(r) => r,
            Err(e) => return CallToolResult::error(format!("Search failed: {e}")),
        };

        let output = json!({
            "query": query,
            "count": results.len(),
            "entries": results.iter().map(|e| json!({
                "id": e.id,
                "category": e.category,
                "title": e.title,
                "content": e.content,
                "tags": e.tags,
                "created_at": e.created_at.to_rfc3339(),
            })).collect::<Vec<_>>()
        });

        return CallToolResult::text(serde_json::to_string_pretty(&output).unwrap_or_default());
    }

    // Category filter mode
    let category = args.get("category").and_then(|v| v.as_str());
    let entries = match knowledge_mgr.list(category) {
        Ok(e) => e,
        Err(e) => return CallToolResult::error(format!("Failed to list knowledge: {e}")),
    };

    // Optionally generate SKILL.md files
    if args
        .get("generate_skills")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        match knowledge_mgr.generate_all_skills() {
            Ok(generated) => {
                if !generated.is_empty() {
                    eprintln!(
                        "[forge-mcp] Generated SKILL.md files: {}",
                        generated.join(", ")
                    );
                }
            }
            Err(e) => eprintln!("[forge-mcp] Failed to generate skills: {e}"),
        }
    }

    let output = json!({
        "category": category.unwrap_or("all"),
        "count": entries.len(),
        "entries": entries.iter().map(|e| json!({
            "id": e.id,
            "category": e.category,
            "title": e.title,
            "content": e.content,
            "tags": e.tags,
            "source": e.source,
            "task_id": e.task_id,
            "created_at": e.created_at.to_rfc3339(),
        })).collect::<Vec<_>>()
    });

    CallToolResult::text(serde_json::to_string_pretty(&output).unwrap_or_default())
}

/// Create the configured brain from state.json
fn get_configured_brain(forge_dir: &Path) -> Box<dyn crate::brain::ForgeBrain> {
    let state_mgr = StateManager::new(forge_dir);
    if let Ok(state) = state_mgr.load() {
        match state.brain.provider.as_str() {
            "openai" => {
                let model = state.brain.model.as_deref().unwrap_or("gpt-4.1");
                Box::new(crate::brain::openai::OpenAIBrain::new(model))
            }
            _ => Box::new(RuleBasedBrain),
        }
    } else {
        Box::new(RuleBasedBrain)
    }
}

fn handle_check_drift(project_root: &Path) -> CallToolResult {
    let checker = GovernanceChecker::new(project_root);
    let forge_dir = project_root.join(".forge");
    let brain = get_configured_brain(&forge_dir);

    let report = checker.full_check(brain.as_ref());

    let drift_json = match &report.drift {
        Some(drift) => json!({
            "vision_alignment": drift.vision_alignment,
            "explanation": drift.explanation,
            "completed_tasks": drift.completed_tasks,
            "total_tasks": drift.total_tasks,
        }),
        None => json!({
            "vision_alignment": null,
            "explanation": "No vision document (SPEC.md or plan.md) found — cannot evaluate drift",
        }),
    };

    // Log governance event
    let forge_dir = project_root.join(".forge");
    let event_logger = EventLogger::new(&forge_dir);
    if let Err(e) = event_logger.log(&ForgeEvent::new(
        EventType::GovernanceCheck,
        format!("Drift check: {}", report.summary),
    )) {
        eprintln!("[forge-orchestrator] event log error: {e}");
    }

    let output = json!({
        "drift": drift_json,
        "summary": report.summary,
    });

    CallToolResult::text(serde_json::to_string_pretty(&output).unwrap_or_default())
}

fn handle_get_health(project_root: &Path) -> CallToolResult {
    let checker = GovernanceChecker::new(project_root);
    let forge_dir = project_root.join(".forge");
    let brain = get_configured_brain(&forge_dir);

    let report = checker.full_check(brain.as_ref());

    // Log governance event
    let forge_dir = project_root.join(".forge");
    let event_logger = EventLogger::new(&forge_dir);
    if let Err(e) = event_logger.log(&ForgeEvent::new(
        EventType::GovernanceCheck,
        format!("Health check: {}", report.summary),
    )) {
        eprintln!("[forge-orchestrator] event log error: {e}");
    }

    let output = json!({
        "health_score": report.health_score,
        "summary": report.summary,
        "findings": report.findings.iter().map(|f| json!({
            "category": f.category,
            "severity": f.severity,
            "message": f.message,
            "suggestion": f.suggestion,
        })).collect::<Vec<_>>(),
        "drift": report.drift.map(|d| json!({
            "vision_alignment": d.vision_alignment,
            "explanation": d.explanation,
            "completed_tasks": d.completed_tasks,
            "total_tasks": d.total_tasks,
        })),
    });

    CallToolResult::text(serde_json::to_string_pretty(&output).unwrap_or_default())
}

fn handle_get_events(args: &Value, forge_dir: &Path) -> CallToolResult {
    let event_logger = EventLogger::new(forge_dir);

    let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

    let task_filter = args.get("task_id").and_then(|v| v.as_str());
    let type_filter = args.get("event_type").and_then(|v| v.as_str());

    let events = match event_logger.read_recent(count.max(500)) {
        Ok(e) => e,
        Err(e) => return CallToolResult::error(format!("Failed to read events: {e}")),
    };

    let filtered: Vec<_> = events
        .into_iter()
        .filter(|e| {
            if let Some(tid) = task_filter
                && e.task_id.as_deref() != Some(tid)
            {
                return false;
            }
            if let Some(et) = type_filter {
                let type_str = serde_json::to_string(&e.event_type).unwrap_or_default();
                let type_str = type_str.trim_matches('"');
                if type_str != et {
                    return false;
                }
            }
            true
        })
        .collect();

    // Take only the last `count` after filtering
    let start = filtered.len().saturating_sub(count);
    let result: Vec<_> = filtered[start..]
        .iter()
        .map(|e| {
            json!({
                "timestamp": e.timestamp.to_rfc3339(),
                "event_type": e.event_type,
                "task_id": e.task_id,
                "agent": e.agent,
                "message": e.message,
                "duration_ms": e.duration_ms,
                "exit_code": e.exit_code,
            })
        })
        .collect();

    CallToolResult::text(serde_json::to_string_pretty(&result).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_returns_eleven() {
        let tools = list_tools();
        assert_eq!(tools.len(), 11);
        assert_eq!(tools[0].name, "forge_get_tasks");
        assert_eq!(tools[1].name, "forge_claim_task");
        assert_eq!(tools[2].name, "forge_complete_task");
        assert_eq!(tools[3].name, "forge_get_state");
        assert_eq!(tools[4].name, "forge_get_plan");
        assert_eq!(tools[5].name, "forge_capture_knowledge");
        assert_eq!(tools[6].name, "forge_get_knowledge");
        assert_eq!(tools[7].name, "forge_check_drift");
        assert_eq!(tools[8].name, "forge_get_health");
        assert_eq!(tools[9].name, "forge_get_events");
        assert_eq!(tools[10].name, "forge_set_project");
    }

    #[test]
    fn test_call_unknown_tool() {
        let result = call_tool("nonexistent", &json!({}), Path::new("/tmp/fake"));
        assert!(result.is_error.unwrap_or(false));
    }

    #[test]
    fn test_call_tool_without_forge_init() {
        let result = call_tool(
            "forge_get_tasks",
            &json!({}),
            Path::new("/tmp/no-forge-here"),
        );
        assert!(result.is_error.unwrap_or(false));
        assert!(result.content[0].text.contains("not initialized"));
    }
}
