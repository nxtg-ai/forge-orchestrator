use crate::adapters::ToolAdapter;
use crate::adapters::claude::ClaudeAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, TaskManager, TaskStatus};
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, task_id: &str, agent_name: &str) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    let task_mgr = TaskManager::new(&forge_dir);
    let state_mgr = StateManager::new(&forge_dir);
    let event_logger = EventLogger::new(&forge_dir);

    // Load task
    let mut task = match task_mgr.get_task(task_id) {
        Ok(t) => t,
        Err(_) => {
            println!(
                "{} Task {} not found in .forge/tasks/",
                "✗".red(),
                task_id.cyan()
            );
            return Ok(());
        }
    };

    // Parse agent type
    let agent_type: AgentType = agent_name.parse()?;

    // Check for file conflicts
    if !task.locked_files.is_empty() {
        let conflicts = state_mgr.check_file_conflicts(&task.locked_files)?;
        if !conflicts.is_empty() {
            println!(
                "{} File conflict detected! The following files are locked:",
                "✗".red()
            );
            for conflict in &conflicts {
                println!(
                    "  {} locked by task {} ({})",
                    conflict.files.join(", ").yellow(),
                    conflict.task_id.cyan(),
                    conflict.agent.to_string().dimmed()
                );
            }
            return Ok(());
        }
    }

    // Update task status
    task.status = TaskStatus::InProgress;
    task.assigned_to = Some(agent_type.clone());
    task.updated_at = chrono::Utc::now();
    task_mgr.update_task(&task)?;

    // Lock files
    if !task.locked_files.is_empty() {
        state_mgr.lock_files(task_id, agent_type.clone(), task.locked_files.clone())?;
    }

    // Log start event
    event_logger.log(
        &ForgeEvent::new(EventType::TaskStarted, format!("Task {task_id} started"))
            .with_task(task_id)
            .with_agent(agent_type.clone()),
    )?;

    println!(
        "{} Executing {} via {} headless mode...",
        "→".cyan(),
        task_id.bold(),
        agent_name.cyan()
    );
    println!();

    // Execute via adapter
    let result = match agent_type {
        AgentType::Claude => {
            let adapter = ClaudeAdapter;
            adapter.execute_headless(&task, project_root)?
        }
        _ => {
            println!(
                "{} Only Claude adapter is implemented in v0.1. Codex and Gemini coming soon.",
                "!".yellow()
            );
            return Ok(());
        }
    };

    // Save result
    let results_dir = forge_dir.join("results");
    std::fs::create_dir_all(&results_dir)?;
    std::fs::write(results_dir.join(format!("{task_id}.txt")), &result.output)?;

    // Update task status based on result
    if result.success {
        task.status = TaskStatus::Completed;
        task.completed_at = Some(chrono::Utc::now());
        println!("{} Task {} completed successfully!", "✓".green(), task_id);

        event_logger.log(
            &ForgeEvent::new(
                EventType::TaskCompleted,
                format!("Task {task_id} completed (exit code: {})", result.exit_code),
            )
            .with_task(task_id)
            .with_agent(agent_type),
        )?;
    } else {
        task.status = TaskStatus::Failed;
        println!(
            "{} Task {} failed (exit code: {})",
            "✗".red(),
            task_id,
            result.exit_code
        );

        event_logger.log(
            &ForgeEvent::new(
                EventType::TaskFailed,
                format!("Task {task_id} failed (exit code: {})", result.exit_code),
            )
            .with_task(task_id)
            .with_agent(agent_type),
        )?;
    }

    task.updated_at = chrono::Utc::now();
    task_mgr.update_task(&task)?;

    // Unlock files
    state_mgr.unlock_files(task_id)?;

    // Show output preview
    println!();
    println!("{}", "Output (first 500 chars):".dimmed());
    let preview: String = result.output.chars().take(500).collect();
    println!("{preview}");
    println!();
    println!(
        "Full output saved to: {}",
        format!(".forge/results/{task_id}.txt").cyan()
    );

    Ok(())
}
