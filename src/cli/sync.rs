use crate::adapters::ToolAdapter;
use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::{StateManager, TaskSummary};
use crate::core::task::{AgentType, TaskManager, TaskStatus};
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    println!("{}", "FORGE — Sync & Reconcile".bold());
    println!("{}", "=".repeat(40));
    println!();

    let task_mgr = TaskManager::new(&forge_dir);
    let state_mgr = StateManager::new(&forge_dir);
    let event_logger = EventLogger::new(&forge_dir);

    let tasks = task_mgr.list_tasks()?;

    // Step 1: Reconcile task summary
    println!("  {} Reconciling task summary...", "→".cyan());
    let mut summary = TaskSummary {
        total: tasks.len(),
        ..Default::default()
    };

    let completed_ids: Vec<String> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .map(|t| t.id.clone())
        .collect();

    for task in &tasks {
        match task.status {
            TaskStatus::Pending => {
                if task.is_blocked(&completed_ids) {
                    summary.blocked += 1;
                } else {
                    summary.pending += 1;
                }
            }
            TaskStatus::Blocked => summary.blocked += 1,
            TaskStatus::Assigned | TaskStatus::InProgress => summary.in_progress += 1,
            TaskStatus::Completed => summary.completed += 1,
            TaskStatus::Failed => summary.failed += 1,
        }
    }

    state_mgr.update_task_summary(summary.clone())?;
    println!(
        "  {} Summary: {} total, {} pending, {} active, {} done, {} blocked",
        "✓".green(),
        summary.total,
        summary.pending,
        summary.in_progress,
        summary.completed,
        summary.blocked
    );

    // Step 2: Re-detect tools (picks up newly installed CLIs)
    let fresh_tools = crate::detect::detect_tools();
    state_mgr.update_tools(&fresh_tools)?;

    // Step 3: Render adapter configs for each detected tool
    println!("  {} Rendering adapter configs...", "→".cyan());
    let updated_state = state_mgr.load()?;

    for tool in &fresh_tools {
        if !tool.available {
            continue;
        }
        match tool.agent_type {
            AgentType::Claude => {
                let adapter = ClaudeAdapter;
                adapter.render_config(&updated_state, &tasks, project_root)?;
                println!("    {} CLAUDE.md updated", "✓".green());
            }
            AgentType::Codex => {
                let adapter = CodexAdapter;
                adapter.render_config(&updated_state, &tasks, project_root)?;
                println!("    {} AGENTS.md updated", "✓".green());
            }
            AgentType::Gemini => {
                let adapter = GeminiAdapter;
                adapter.render_config(&updated_state, &tasks, project_root)?;
                println!("    {} GEMINI.md updated", "✓".green());
            }
            AgentType::Any => {}
        }
    }

    // Step 4: Log reconciliation event
    event_logger.log(&ForgeEvent::new(
        EventType::StateReconciled,
        format!(
            "State reconciled: {} tasks ({} done, {} pending)",
            summary.total, summary.completed, summary.pending
        ),
    ))?;

    println!();
    println!("{} Sync complete.", "✓".green().bold());

    Ok(())
}
