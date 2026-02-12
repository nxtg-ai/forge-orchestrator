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
        "┌──────────────────────────────────────────────┐".cyan()
    );
    println!(
        "{}",
        "│  FORGE ORCHESTRATOR STATUS                    │"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "└──────────────────────────────────────────────┘".cyan()
    );
    println!();

    // Project info
    println!("  {} {}", "Project:".dimmed(), state.project_name.bold());
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

    // Compute counts
    let mut in_progress = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;

    for task in &tasks {
        match task.status {
            TaskStatus::Pending | TaskStatus::Blocked => {}
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

    let completed_ids: Vec<String> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .map(|t| t.id.clone())
        .collect();

    // Task Board table
    println!("  {}", "Task Board:".bold());
    if tasks.is_empty() {
        println!("    {} No tasks found. Run {} to create tasks.", "!".yellow(), "forge plan --generate".cyan());
    } else {
        println!(
            "    {:<8} {:<8} {:<12} {:<10} {:<10} {:<32} {}",
            "ID".dimmed(),
            "Phase".dimmed(),
            "Status".dimmed(),
            "Agent".dimmed(),
            "Type".dimmed(),
            "Title".dimmed(),
            "Deps".dimmed(),
        );
        println!("    {}", "─".repeat(96));

        for task in &tasks {
            let status_str = match task.status {
                TaskStatus::Pending => {
                    if task.is_blocked(&completed_ids) {
                        "⏳ Blocked".yellow().to_string()
                    } else {
                        "○ Ready".white().to_string()
                    }
                }
                TaskStatus::Assigned | TaskStatus::InProgress => "⚡ Running".cyan().to_string(),
                TaskStatus::Completed => "✓ Done".green().to_string(),
                TaskStatus::Failed => "✗ Failed".red().to_string(),
                TaskStatus::Blocked => "⏳ Blocked".yellow().to_string(),
            };

            let agent_str = task
                .assigned_to
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "-".into());

            let type_str = task.task_type.as_deref().unwrap_or("-");

            let title = truncate_title(&task.title, 30);

            let phase_str = task
                .phase
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "build".into());

            let deps = if task.depends_on.is_empty() {
                "-".to_string()
            } else {
                task.depends_on
                    .iter()
                    .map(|d| {
                        if completed_ids.contains(d) {
                            d.green().to_string()
                        } else {
                            d.yellow().to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            // Indent subtasks (V-xxx, fix tasks with parent)
            let id_display = if task.parent_task.is_some() {
                format!(" {}", task.id).cyan().to_string()
            } else {
                task.id.cyan().to_string()
            };

            println!(
                "    {:<8} {:<8} {:<12} {:<10} {:<10} {:<32} {}",
                id_display,
                phase_str.dimmed(),
                status_str,
                agent_str,
                type_str.dimmed(),
                title,
                deps
            );
        }
    }
    println!();

    // Summary footer with ready/blocked breakdown
    let blocked_count = tasks
        .iter()
        .filter(|t| {
            matches!(t.status, TaskStatus::Pending | TaskStatus::Blocked)
                && t.is_blocked(&completed_ids)
        })
        .count();
    let ready_count = tasks
        .iter()
        .filter(|t| {
            matches!(t.status, TaskStatus::Pending)
                && !t.is_blocked(&completed_ids)
        })
        .count();

    println!(
        "    {} total | {} ready | {} running | {} done | {} failed | {} blocked",
        total.to_string().bold(),
        ready_count.to_string().white(),
        in_progress.to_string().cyan(),
        completed.to_string().green(),
        failed.to_string().red(),
        blocked_count.to_string().yellow(),
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
            let task_str = event.task_id.as_deref().unwrap_or("");
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

/// Truncate a title to fit in the task board column (max 32 chars).
fn truncate_title(title: &str, max_len: usize) -> String {
    if title.len() > max_len {
        format!("{}...", &title[..max_len.saturating_sub(3)])
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::{AgentType, Task};
    use chrono::Utc;

    fn make_task(id: &str, status: TaskStatus, deps: Vec<String>) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            title: format!("Task {id}"),
            description: String::new(),
            status,
            assigned_to: Some(AgentType::Claude),
            task_type: Some("implement".to_string()),
            depends_on: deps,
            locked_files: vec![],
            acceptance_criteria: vec![],
            created_at: now,
            updated_at: now,
            completed_at: None,
            plan_version: None,
            parent_task: None,
            phase: None,
            retry_count: 0,
        }
    }

    #[test]
    fn test_truncate_title_short() {
        assert_eq!(truncate_title("Hello", 32), "Hello");
    }

    #[test]
    fn test_truncate_title_exact() {
        let title = "a".repeat(32);
        assert_eq!(truncate_title(&title, 32), title);
    }

    #[test]
    fn test_truncate_title_long() {
        let title = "a".repeat(50);
        let result = truncate_title(&title, 32);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_ready_vs_blocked_counting() {
        let tasks = vec![
            make_task("T-001", TaskStatus::Completed, vec![]),
            make_task("T-002", TaskStatus::Pending, vec!["T-001".into()]),  // ready (T-001 done)
            make_task("T-003", TaskStatus::Pending, vec!["T-002".into()]),  // blocked (T-002 not done)
            make_task("T-004", TaskStatus::Pending, vec![]),                 // ready (no deps)
        ];

        let completed_ids: Vec<String> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        let ready_count = tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Pending) && !t.is_blocked(&completed_ids)
            })
            .count();

        let blocked_count = tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Pending | TaskStatus::Blocked)
                    && t.is_blocked(&completed_ids)
            })
            .count();

        assert_eq!(ready_count, 2);   // T-002 and T-004
        assert_eq!(blocked_count, 1); // T-003
    }

    #[test]
    fn test_status_display_blocked_pending_distinction() {
        let completed_ids = vec!["T-001".to_string()];
        let blocked_task = make_task("T-003", TaskStatus::Pending, vec!["T-002".into()]);
        let ready_task = make_task("T-002", TaskStatus::Pending, vec!["T-001".into()]);

        assert!(blocked_task.is_blocked(&completed_ids));
        assert!(!ready_task.is_blocked(&completed_ids));
    }

    #[test]
    fn test_empty_task_board_counts() {
        let tasks: Vec<Task> = vec![];
        let completed_ids: Vec<String> = vec![];

        let ready_count = tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Pending) && !t.is_blocked(&completed_ids)
            })
            .count();

        let blocked_count = tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Pending | TaskStatus::Blocked)
                    && t.is_blocked(&completed_ids)
            })
            .count();

        assert_eq!(ready_count, 0);
        assert_eq!(blocked_count, 0);
    }
}
