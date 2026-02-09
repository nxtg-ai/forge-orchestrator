use crate::core::event::EventLogger;
use crate::core::state::StateManager;
use crate::core::task::{TaskManager, TaskStatus};
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, event_count: usize) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    let state_mgr = StateManager::new(&forge_dir);
    let task_mgr = TaskManager::new(&forge_dir);
    let event_logger = EventLogger::new(&forge_dir);

    let state = state_mgr.load()?;
    let tasks = task_mgr.list_tasks()?;
    let recent_events = event_logger.read_recent(event_count)?;

    // Header
    println!();
    println!(
        "{}",
        "┌──────────────────────────────────────────────┐"
            .cyan()
    );
    println!(
        "{}",
        "│  FORGE ORCHESTRATOR STATUS                    │"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "└──────────────────────────────────────────────┘"
            .cyan()
    );
    println!();

    // Project info
    println!(
        "  {} {}",
        "Project:".dimmed(),
        state.project_name.bold()
    );
    println!(
        "  {} {}",
        "Updated:".dimmed(),
        state.updated_at.format("%Y-%m-%d %H:%M UTC")
    );
    println!(
        "  {} {}",
        "Brain:  ".dimmed(),
        state.brain.provider.yellow()
    );
    println!();

    // Tool status
    println!("  {}", "Detected Tools:".bold());
    if state.tools.is_empty() {
        println!("    {} No AI tools detected", "!".yellow());
    } else {
        for tool in &state.tools {
            let status = if tool.available {
                "●".green().to_string()
            } else {
                "○".red().to_string()
            };
            println!("    {status} {}", tool.name);
        }
    }
    println!();

    // Task summary
    let mut pending = 0;
    let mut in_progress = 0;
    let mut completed = 0;
    let mut failed = 0;

    for task in &tasks {
        match task.status {
            TaskStatus::Pending | TaskStatus::Blocked => pending += 1,
            TaskStatus::Assigned | TaskStatus::InProgress => in_progress += 1,
            TaskStatus::Completed => completed += 1,
            TaskStatus::Failed => failed += 1,
        }
    }

    let total = tasks.len();
    let progress = if total > 0 {
        (completed as f64 / total as f64 * 100.0) as usize
    } else {
        0
    };

    println!("  {}", "Task Summary:".bold());
    println!(
        "    Total: {}  Pending: {}  Active: {}  Done: {}  Failed: {}",
        total.to_string().bold(),
        pending.to_string().yellow(),
        in_progress.to_string().cyan(),
        completed.to_string().green(),
        failed.to_string().red()
    );

    // Progress bar
    let bar_width = 30;
    let filled = (progress * bar_width) / 100;
    let empty = bar_width - filled;
    let bar = format!(
        "    [{}{}] {}%",
        "█".repeat(filled).green(),
        "░".repeat(empty),
        progress
    );
    println!("{bar}");
    println!();

    // Active tasks
    let active_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Assigned | TaskStatus::InProgress
            )
        })
        .collect();

    if !active_tasks.is_empty() {
        println!("  {}", "Active Tasks:".bold());
        for task in &active_tasks {
            let agent = task
                .assigned_to
                .as_ref()
                .map(|a| format!("[{a}]"))
                .unwrap_or_else(|| "[?]".into());
            println!(
                "    {} {} {}",
                agent.cyan(),
                task.id.dimmed(),
                task.title
            );
        }
        println!();
    }

    // File locks
    if !state.active_locks.is_empty() {
        println!("  {}", "Active Locks:".bold());
        for (task_id, lock) in &state.active_locks {
            println!(
                "    {} → {} files locked by {}",
                task_id.dimmed(),
                lock.files.len(),
                lock.agent.to_string().cyan()
            );
        }
        println!();
    }

    // Recent events
    if !recent_events.is_empty() {
        println!("  {}", "Recent Events:".bold());
        for event in &recent_events {
            let time = event.timestamp.format("%H:%M");
            let task_str = event
                .task_id
                .as_deref()
                .unwrap_or("");
            println!(
                "    {} {} {}",
                time.to_string().dimmed(),
                task_str.dimmed(),
                event.message
            );
        }
        println!();
    }

    Ok(())
}
