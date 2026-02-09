use crate::adapters::ToolAdapter;
use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskStatus};
use crate::detect;
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Autonomous orchestration — runs all tasks respecting dependencies,
/// one thread per agent type, auto-claim/complete/retry.
pub fn execute(project_root: &Path, agent_filter: Option<&str>) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    println!("{}", "FORGE — Autonomous Orchestration".bold());
    println!("{}", "=".repeat(40));
    println!();

    // Detect available tools
    let detected = detect::detect_tools();
    let available_agents: Vec<AgentType> = detected
        .iter()
        .filter(|t| t.available)
        .map(|t| t.agent_type.clone())
        .collect();

    if available_agents.is_empty() {
        println!(
            "{} No AI tools detected. Install claude, codex, or gemini.",
            "✗".red()
        );
        return Ok(());
    }

    // Filter agents if requested
    let target_agents: Vec<AgentType> = if let Some(filter) = agent_filter {
        let agent: AgentType = filter.parse()?;
        if !available_agents.contains(&agent) {
            println!(
                "{} Agent '{}' is not installed on this machine.",
                "✗".red(),
                filter
            );
            return Ok(());
        }
        vec![agent]
    } else {
        available_agents.clone()
    };

    // Show what we're working with
    let task_mgr = TaskManager::new(&forge_dir);
    let tasks = task_mgr.list_tasks()?;
    let total = tasks.len();
    let done = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();

    println!(
        "  {} agents: {}",
        "→".cyan(),
        target_agents
            .iter()
            .map(|a| a.to_string().green().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  {} tasks:  {} total, {} done, {} remaining",
        "→".cyan(),
        total,
        done,
        total - done
    );
    println!();

    if total == done {
        println!("{} All tasks already complete!", "✓".green().bold());
        return Ok(());
    }

    // Run orchestration
    if target_agents.len() == 1 {
        // Single agent — run sequentially
        run_agent_loop(project_root, &forge_dir, &target_agents[0])?;
    } else {
        // Multiple agents — run in parallel (one thread per agent)
        run_parallel(project_root, &forge_dir, &target_agents)?;
    }

    // Final status
    let tasks = task_mgr.list_tasks()?;
    let completed = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let failed = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();

    println!();
    println!("{}", "=".repeat(40));
    println!(
        "  {} Done. {}/{} completed, {} failed.",
        if failed == 0 {
            "✓".green()
        } else {
            "!".yellow()
        },
        completed,
        total,
        failed
    );

    Ok(())
}

/// Run multiple agents in parallel, each managing its own task queue.
fn run_parallel(project_root: &Path, forge_dir: &Path, agents: &[AgentType]) -> anyhow::Result<()> {
    let project_root = project_root.to_path_buf();
    let forge_dir = forge_dir.to_path_buf();

    // Shared lock for coordinating task state access between threads
    let lock = Arc::new(Mutex::new(()));

    let handles: Vec<_> = agents
        .iter()
        .map(|agent| {
            let pr = project_root.clone();
            let fd = forge_dir.clone();
            let agent = agent.clone();
            let lock = Arc::clone(&lock);

            std::thread::spawn(move || -> anyhow::Result<()> {
                run_agent_loop_with_lock(&pr, &fd, &agent, &lock)
            })
        })
        .collect();

    let mut errors = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(_) => errors.push("Agent thread panicked".into()),
        }
    }

    if !errors.is_empty() {
        eprintln!();
        for err in &errors {
            eprintln!("  {} {}", "Error:".red(), err);
        }
    }

    Ok(())
}

/// Single-agent loop — find next task, claim, execute, complete, repeat.
fn run_agent_loop(project_root: &Path, forge_dir: &Path, agent: &AgentType) -> anyhow::Result<()> {
    let lock = Arc::new(Mutex::new(()));
    run_agent_loop_with_lock(project_root, forge_dir, agent, &lock)
}

/// Agent loop with shared coordination lock.
fn run_agent_loop_with_lock(
    project_root: &Path,
    forge_dir: &Path,
    agent: &AgentType,
    lock: &Arc<Mutex<()>>,
) -> anyhow::Result<()> {
    let tag = format!("[{}]", agent).cyan();

    loop {
        // Under lock: find next available task for this agent
        let next_task = {
            let _guard = lock.lock().unwrap();
            find_next_task(forge_dir, agent)?
        };

        let task = match next_task {
            Some(t) => t,
            None => {
                // Check if there are pending tasks (could unblock later from another agent)
                let task_mgr = TaskManager::new(forge_dir);
                let tasks = task_mgr.list_tasks()?;
                let my_pending: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
                        t.assigned_to.as_ref() == Some(agent)
                            && (t.status == TaskStatus::Pending || t.status == TaskStatus::Blocked)
                    })
                    .collect();

                if my_pending.is_empty() {
                    println!("  {tag} All my tasks are done.");
                    break;
                }

                // Wait for other agents to unblock our tasks
                println!(
                    "  {tag} {} tasks blocked, waiting for dependencies...",
                    my_pending.len()
                );
                std::thread::sleep(std::time::Duration::from_secs(10));
                continue;
            }
        };

        // Claim the task (under lock)
        {
            let _guard = lock.lock().unwrap();
            claim_task(forge_dir, &task, agent)?;
        }

        println!(
            "  {tag} {} Executing {}: {}",
            "→".cyan(),
            task.id.bold(),
            task.title
        );

        let started = Instant::now();

        // Execute (NOT under lock — this takes minutes)
        let result = execute_task(&task, agent, project_root);
        let elapsed = started.elapsed();

        // Complete or fail (under lock)
        {
            let _guard = lock.lock().unwrap();
            match result {
                Ok(output) if output.success => {
                    complete_task(forge_dir, &task, agent)?;
                    println!(
                        "  {tag} {} {} completed ({:.0}s)",
                        "✓".green(),
                        task.id.bold(),
                        elapsed.as_secs_f64()
                    );
                }
                Ok(output) => {
                    fail_task(forge_dir, &task)?;
                    save_result(forge_dir, &task.id, &output.output)?;
                    println!(
                        "  {tag} {} {} failed (exit code: {}, {:.0}s)",
                        "✗".red(),
                        task.id,
                        output.exit_code,
                        elapsed.as_secs_f64()
                    );
                }
                Err(e) => {
                    fail_task(forge_dir, &task)?;
                    println!(
                        "  {tag} {} {} error: {} ({:.0}s)",
                        "✗".red(),
                        task.id,
                        e,
                        elapsed.as_secs_f64()
                    );
                }
            }
        }
    }

    Ok(())
}

/// Find the next available task for a specific agent.
fn find_next_task(forge_dir: &Path, agent: &AgentType) -> anyhow::Result<Option<Task>> {
    let task_mgr = TaskManager::new(forge_dir);
    let completed = task_mgr.get_completed_task_ids()?;
    let tasks = task_mgr.list_tasks()?;

    Ok(tasks.into_iter().find(|t| {
        t.status == TaskStatus::Pending
            && t.assigned_to.as_ref() == Some(agent)
            && !t.is_blocked(&completed)
    }))
}

/// Claim a task — update status, lock files.
fn claim_task(forge_dir: &Path, task: &Task, agent: &AgentType) -> anyhow::Result<()> {
    let task_mgr = TaskManager::new(forge_dir);
    let state_mgr = StateManager::new(forge_dir);
    let event_logger = EventLogger::new(forge_dir);

    let mut updated = task.clone();
    updated.status = TaskStatus::InProgress;
    updated.assigned_to = Some(agent.clone());
    updated.updated_at = chrono::Utc::now();
    task_mgr.update_task(&updated)?;

    // Lock files
    if !task.locked_files.is_empty() {
        state_mgr.lock_files(&task.id, agent.clone(), task.locked_files.clone())?;
    }

    event_logger.log(
        &ForgeEvent::new(
            EventType::TaskStarted,
            format!("Task {} claimed by {agent}", task.id),
        )
        .with_task(&task.id)
        .with_agent(agent.clone()),
    )?;

    Ok(())
}

/// Mark a task as completed — update status, unlock files.
fn complete_task(forge_dir: &Path, task: &Task, agent: &AgentType) -> anyhow::Result<()> {
    let task_mgr = TaskManager::new(forge_dir);
    let state_mgr = StateManager::new(forge_dir);
    let event_logger = EventLogger::new(forge_dir);

    let mut updated = task.clone();
    updated.status = TaskStatus::Completed;
    updated.completed_at = Some(chrono::Utc::now());
    updated.updated_at = chrono::Utc::now();
    task_mgr.update_task(&updated)?;

    state_mgr.unlock_files(&task.id)?;

    event_logger.log(
        &ForgeEvent::new(
            EventType::TaskCompleted,
            format!("Task {} completed by {agent}", task.id),
        )
        .with_task(&task.id)
        .with_agent(agent.clone()),
    )?;

    Ok(())
}

/// Mark a task as failed.
fn fail_task(forge_dir: &Path, task: &Task) -> anyhow::Result<()> {
    let task_mgr = TaskManager::new(forge_dir);
    let state_mgr = StateManager::new(forge_dir);

    let mut updated = task.clone();
    updated.status = TaskStatus::Failed;
    updated.updated_at = chrono::Utc::now();
    task_mgr.update_task(&updated)?;

    state_mgr.unlock_files(&task.id)?;

    Ok(())
}

/// Save task output to .forge/results/
fn save_result(forge_dir: &Path, task_id: &str, output: &str) -> anyhow::Result<()> {
    let results_dir = PathBuf::from(forge_dir).join("results");
    std::fs::create_dir_all(&results_dir)?;
    std::fs::write(results_dir.join(format!("{task_id}.txt")), output)?;
    Ok(())
}

/// Execute a task via the appropriate CLI adapter.
fn execute_task(
    task: &Task,
    agent: &AgentType,
    project_root: &Path,
) -> anyhow::Result<crate::adapters::ExecutionResult> {
    match agent {
        AgentType::Claude => ClaudeAdapter.execute_headless(task, project_root),
        AgentType::Codex => CodexAdapter.execute_headless(task, project_root),
        AgentType::Gemini => GeminiAdapter.execute_headless(task, project_root),
        AgentType::Any => ClaudeAdapter.execute_headless(task, project_root),
    }
}
