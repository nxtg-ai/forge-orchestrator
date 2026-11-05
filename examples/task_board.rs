//! Print a project's task board, grouped by status, straight from `.forge/` (no `forge` process).
//!
//! ```sh
//! cargo run --example task_board -- path/to/project
//! ```
use forge_orchestrator::core::task::{TaskManager, TaskStatus};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let project = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let tasks = TaskManager::new(project.join(".forge")).list_tasks()?;
    if tasks.is_empty() {
        println!("no tasks in {}", project.display());
        return Ok(());
    }
    for status in [
        TaskStatus::InProgress,
        TaskStatus::Assigned,
        TaskStatus::Pending,
        TaskStatus::Blocked,
        TaskStatus::Failed,
        TaskStatus::Completed,
    ] {
        let group: Vec<_> = tasks.iter().filter(|t| t.status == status).collect();
        if group.is_empty() {
            continue;
        }
        println!("{status:?} ({})", group.len());
        for t in group {
            let who = t
                .assigned_to
                .as_ref()
                .map(|a| format!(" [{a}]"))
                .unwrap_or_default();
            println!("  {} {}{who}", t.id, t.title);
        }
    }
    Ok(())
}
