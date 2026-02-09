use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::plan::PlanManager;
use crate::core::state::StateManager;
use crate::core::task::{AgentType, TaskManager, TaskStatus};
use serde_json::{json, Value};
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
    ]
}

/// Dispatch a tool call to the appropriate handler
pub fn call_tool(
    name: &str,
    args: &Value,
    project_root: &Path,
) -> CallToolResult {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        return CallToolResult::error(
            "Forge is not initialized. Run `forge init` in this project first.",
        );
    }

    match name {
        "forge_get_tasks" => handle_get_tasks(args, &forge_dir),
        "forge_claim_task" => handle_claim_task(args, &forge_dir),
        "forge_complete_task" => handle_complete_task(args, &forge_dir),
        "forge_get_state" => handle_get_state(&forge_dir),
        "forge_get_plan" => handle_get_plan(&forge_dir),
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
    if !task.locked_files.is_empty() {
        if let Err(e) = state_mgr.lock_files(task_id, agent.clone(), task.locked_files.clone()) {
            return CallToolResult::error(format!("Failed to lock files: {e}"));
        }
    }

    // Log event
    let _ = event_logger.log(
        &ForgeEvent::new(
            EventType::TaskAssigned,
            format!("Task {task_id} claimed by {agent}"),
        )
        .with_task(task_id)
        .with_agent(agent.clone()),
    );

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
    let _ = state_mgr.unlock_files(task_id);

    // Log event
    let _ = event_logger.log(
        &ForgeEvent::new(
            EventType::TaskCompleted,
            format!("Task {task_id} completed: {result_summary}"),
        )
        .with_task(task_id),
    );

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
        CallToolResult::text("No plan exists yet. Use `forge plan --generate` to create one from SPEC.md.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_returns_five() {
        let tools = list_tools();
        assert_eq!(tools.len(), 5);
        assert_eq!(tools[0].name, "forge_get_tasks");
        assert_eq!(tools[1].name, "forge_claim_task");
        assert_eq!(tools[2].name, "forge_complete_task");
        assert_eq!(tools[3].name, "forge_get_state");
        assert_eq!(tools[4].name, "forge_get_plan");
    }

    #[test]
    fn test_call_unknown_tool() {
        let result = call_tool("nonexistent", &json!({}), Path::new("/tmp/fake"));
        assert!(result.is_error.unwrap_or(false));
    }

    #[test]
    fn test_call_tool_without_forge_init() {
        let result = call_tool("forge_get_tasks", &json!({}), Path::new("/tmp/no-forge-here"));
        assert!(result.is_error.unwrap_or(false));
        assert!(result.content[0].text.contains("not initialized"));
    }
}
