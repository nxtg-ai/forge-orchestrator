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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Max retries for transient failures (rate limits, timeouts).
const MAX_RETRIES: usize = 3;

/// Known transient error strings that should trigger a retry.
const TRANSIENT_ERRORS: &[&str] = &[
    "credit balance is too low",
    "rate limit",
    "rate_limit",
    "too many requests",
    "429",
    "timeout",
    "timed out",
    "connection reset",
    "connection refused",
    "ECONNRESET",
    "ECONNREFUSED",
    "ETIMEDOUT",
    "server error",
    "internal server error",
    "502",
    "503",
    "overloaded",
];

/// Shared orchestration stats.
struct OrcStats {
    completed: AtomicUsize,
    failed: AtomicUsize,
    retried: AtomicUsize,
    total: usize,
}

/// Autonomous orchestration — runs all tasks respecting dependencies,
/// one thread per agent type, with retry, progress tracking, and auto-sync.
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

    let start_time = Instant::now();

    // ── Header ──────────────────────────────────────────────────
    println!();
    println!(
        "  {}",
        "╔══════════════════════════════════════════════╗"
            .bright_cyan()
    );
    println!(
        "  {}  {}  {}",
        "║".bright_cyan(),
        "FORGE — Autonomous Orchestration".bold().white(),
        "       ║".bright_cyan()
    );
    println!(
        "  {}",
        "╚══════════════════════════════════════════════╝"
            .bright_cyan()
    );
    println!();

    // ── Detect agents ───────────────────────────────────────────
    let detected = detect::detect_tools();
    let available_agents: Vec<AgentType> = detected
        .iter()
        .filter(|t| t.available)
        .map(|t| t.agent_type.clone())
        .collect();

    if available_agents.is_empty() {
        println!(
            "  {} No AI tools detected. Install claude, codex, or gemini.",
            "✗".red()
        );
        return Ok(());
    }

    let target_agents: Vec<AgentType> = if let Some(filter) = agent_filter {
        let agent: AgentType = filter.parse()?;
        if !available_agents.contains(&agent) {
            println!(
                "  {} Agent '{}' is not installed on this machine.",
                "✗".red(),
                filter
            );
            return Ok(());
        }
        vec![agent]
    } else {
        available_agents.clone()
    };

    // ── Task inventory ──────────────────────────────────────────
    let task_mgr = TaskManager::new(&forge_dir);
    let tasks = task_mgr.list_tasks()?;
    let total = tasks.len();
    let already_done = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();

    // Count tasks per agent
    for agent in &target_agents {
        let count = tasks
            .iter()
            .filter(|t| t.assigned_to.as_ref() == Some(agent))
            .count();
        let done = tasks
            .iter()
            .filter(|t| {
                t.assigned_to.as_ref() == Some(agent) && t.status == TaskStatus::Completed
            })
            .count();
        println!(
            "  {} {:<8} {} tasks ({} done)",
            "●".green(),
            agent.to_string(),
            count,
            done
        );
    }
    println!();

    print_progress(already_done, total);

    if total == already_done {
        println!(
            "\n  {} All tasks already complete!",
            "✓".green().bold()
        );
        return Ok(());
    }

    // ── Shared stats ────────────────────────────────────────────
    let stats = Arc::new(OrcStats {
        completed: AtomicUsize::new(already_done),
        failed: AtomicUsize::new(0),
        retried: AtomicUsize::new(0),
        total,
    });

    // ── Run orchestration ───────────────────────────────────────
    let lock = Arc::new(Mutex::new(()));

    if target_agents.len() == 1 {
        run_agent_loop(
            project_root,
            &forge_dir,
            &target_agents[0],
            &lock,
            &stats,
        )?;
    } else {
        run_parallel(project_root, &forge_dir, &target_agents, &stats)?;
    }

    // ── Final report ────────────────────────────────────────────
    let elapsed = start_time.elapsed();
    let tasks = task_mgr.list_tasks()?;
    let final_completed = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let final_failed = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();
    let retries = stats.retried.load(Ordering::Relaxed);

    println!();
    println!(
        "  {}",
        "╔══════════════════════════════════════════════╗"
            .bright_cyan()
    );
    println!(
        "  {}  {}  {}",
        "║".bright_cyan(),
        "Orchestration Complete".bold().white(),
        "                ║".bright_cyan()
    );
    println!(
        "  {}",
        "╚══════════════════════════════════════════════╝"
            .bright_cyan()
    );
    println!();

    println!(
        "  {} {}/{} tasks completed",
        if final_failed == 0 {
            "✓".green()
        } else {
            "!".yellow()
        },
        final_completed,
        total
    );
    if final_failed > 0 {
        println!("  {} {} tasks failed", "✗".red(), final_failed);
    }
    if retries > 0 {
        println!(
            "  {} {} transient retries (recovered)",
            "↻".yellow(),
            retries
        );
    }
    println!(
        "  {} Total time: {}",
        "⏱".dimmed(),
        format_duration(elapsed)
    );

    // Per-agent summary
    println!();
    for agent in &target_agents {
        let agent_completed = tasks
            .iter()
            .filter(|t| {
                t.assigned_to.as_ref() == Some(agent) && t.status == TaskStatus::Completed
            })
            .count();
        let agent_failed = tasks
            .iter()
            .filter(|t| t.assigned_to.as_ref() == Some(agent) && t.status == TaskStatus::Failed)
            .count();
        let agent_total = tasks
            .iter()
            .filter(|t| t.assigned_to.as_ref() == Some(agent))
            .count();

        let bar = if agent_total > 0 {
            let pct = (agent_completed as f64 / agent_total as f64 * 100.0) as usize;
            format!("{}%", pct)
        } else {
            "—".into()
        };

        println!(
            "  {:<8} {} done, {} failed  [{}]",
            format!("[{}]", agent).cyan(),
            agent_completed.to_string().green(),
            if agent_failed > 0 {
                agent_failed.to_string().red().to_string()
            } else {
                "0".dimmed().to_string()
            },
            bar
        );
    }

    // List failed tasks
    let failed_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .collect();
    if !failed_tasks.is_empty() {
        println!();
        println!("  {} Failed tasks:", "✗".red());
        for t in &failed_tasks {
            println!("    {} {}: {}", "·".dimmed(), t.id, t.title.dimmed());
            let result_path = forge_dir.join("results").join(format!("{}.txt", t.id));
            if result_path.exists() {
                println!(
                    "      {} {}",
                    "→".dimmed(),
                    result_path.display().to_string().dimmed()
                );
            }
        }
    }

    // ── Auto-sync ───────────────────────────────────────────────
    println!();
    print!("  {} Syncing state...", "↻".cyan());
    if let Err(e) = crate::cli::sync::execute(project_root) {
        println!(" {}", format!("failed: {e}").red());
    } else {
        println!(" {}", "done".green());
    }

    println!();

    Ok(())
}

/// Print a progress bar.
fn print_progress(done: usize, total: usize) {
    let width = 30;
    let pct = if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    };
    let filled = (pct * width as f64) as usize;
    let empty = width - filled;

    let bar = format!(
        "{}{}",
        "█".repeat(filled).green(),
        "░".repeat(empty).dimmed()
    );
    println!(
        "  [{}] {}/{} ({:.0}%)",
        bar,
        done,
        total,
        pct * 100.0
    );
    println!();
}

/// Format a Duration as human-readable.
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Check if an error output looks transient (retryable).
fn is_transient_error(output: &str) -> bool {
    let lower = output.to_lowercase();
    TRANSIENT_ERRORS.iter().any(|e| lower.contains(e))
}

/// Run multiple agents in parallel, each managing its own task queue.
fn run_parallel(
    project_root: &Path,
    forge_dir: &Path,
    agents: &[AgentType],
    stats: &Arc<OrcStats>,
) -> anyhow::Result<()> {
    let project_root = project_root.to_path_buf();
    let forge_dir = forge_dir.to_path_buf();
    let lock = Arc::new(Mutex::new(()));

    let handles: Vec<_> = agents
        .iter()
        .map(|agent| {
            let pr = project_root.clone();
            let fd = forge_dir.clone();
            let agent = agent.clone();
            let lock = Arc::clone(&lock);
            let stats = Arc::clone(stats);

            std::thread::spawn(move || -> anyhow::Result<()> {
                run_agent_loop(&pr, &fd, &agent, &lock, &stats)
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

/// Agent loop with retry, progress tracking, and shared coordination.
fn run_agent_loop(
    project_root: &Path,
    forge_dir: &Path,
    agent: &AgentType,
    lock: &Arc<Mutex<()>>,
    stats: &Arc<OrcStats>,
) -> anyhow::Result<()> {
    let tag = format!("[{}]", agent).cyan();
    let mut consecutive_waits = 0u32;

    loop {
        // Under lock: find next available task for this agent
        let next_task = {
            let _guard = lock.lock().unwrap();
            find_next_task(forge_dir, agent)?
        };

        let task = match next_task {
            Some(t) => {
                consecutive_waits = 0;
                t
            }
            None => {
                let task_mgr = TaskManager::new(forge_dir);
                let tasks = task_mgr.list_tasks()?;
                let my_remaining: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
                        t.assigned_to.as_ref() == Some(agent)
                            && (t.status == TaskStatus::Pending
                                || t.status == TaskStatus::Blocked)
                    })
                    .collect();

                if my_remaining.is_empty() {
                    println!("  {tag} {} All tasks complete.", "✓".green());
                    break;
                }

                consecutive_waits += 1;

                // Only print every 6th wait (~60s) to reduce noise
                if consecutive_waits % 6 == 1 {
                    println!(
                        "  {tag} {} {} tasks waiting on dependencies...",
                        "⏳".dimmed(),
                        my_remaining.len()
                    );
                }

                // Smart timeout: only give up if NO progress is being made globally.
                // Check if any other agent has completed work recently by comparing
                // our snapshot of completed tasks vs the current count.
                let current_done = stats.completed.load(Ordering::Relaxed);
                let current_failed = stats.failed.load(Ordering::Relaxed);
                let total_finished = current_done + current_failed;

                // If other agents are still working (tasks in_progress exist),
                // or if progress was made recently, keep waiting indefinitely.
                let any_in_progress = {
                    let tasks = task_mgr.list_tasks()?;
                    tasks.iter().any(|t| t.status == TaskStatus::InProgress)
                };

                if any_in_progress {
                    // Reset wait counter — someone is actively working
                    if consecutive_waits > 6 {
                        consecutive_waits = 1;
                    }
                } else if total_finished >= stats.total {
                    // Everything is either done or failed — nothing left to unblock us
                    println!(
                        "  {tag} {} All other tasks finished but dependencies not met.",
                        "!".yellow()
                    );
                    break;
                } else if consecutive_waits > 30 {
                    // No progress for 5 minutes and nothing in_progress
                    println!(
                        "  {tag} {} No progress detected for 5 minutes. Stopping.",
                        "!".yellow()
                    );
                    break;
                }

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
            "  {tag} {} {} {}",
            "→".cyan(),
            task.id.bold(),
            task.title
        );

        // ── Execute with retry ──────────────────────────────────
        let started = Instant::now();
        let mut attempt = 0;
        let mut last_output;
        let mut success = false;

        loop {
            attempt += 1;
            let result = execute_task(&task, agent, project_root);
            let elapsed = started.elapsed();

            match result {
                Ok(output) if output.success => {
                    let _guard = lock.lock().unwrap();
                    complete_task(forge_dir, &task, agent)?;
                    let done = stats.completed.fetch_add(1, Ordering::Relaxed) + 1;
                    println!(
                        "  {tag} {} {} ({}) [{}/{}]",
                        "✓".green(),
                        task.id.bold(),
                        format_duration(elapsed).dimmed(),
                        done,
                        stats.total
                    );
                    success = true;
                    break;
                }
                Ok(output) => {
                    last_output = output.output.clone();
                    save_result(forge_dir, &task.id, &last_output).ok();

                    // Check if retryable
                    if attempt < MAX_RETRIES && is_transient_error(&last_output) {
                        let backoff = std::time::Duration::from_secs(10 * attempt as u64);
                        stats.retried.fetch_add(1, Ordering::Relaxed);
                        println!(
                            "  {tag} {} {} transient error, retry {}/{} in {}s...",
                            "↻".yellow(),
                            task.id,
                            attempt,
                            MAX_RETRIES,
                            backoff.as_secs()
                        );

                        // Reset task to in_progress for retry
                        {
                            let _guard = lock.lock().unwrap();
                            let task_mgr = TaskManager::new(forge_dir);
                            let mut updated = task.clone();
                            updated.status = TaskStatus::InProgress;
                            updated.updated_at = chrono::Utc::now();
                            task_mgr.update_task(&updated)?;
                        }

                        std::thread::sleep(backoff);
                        continue;
                    }

                    // Permanent failure
                    let _guard = lock.lock().unwrap();
                    fail_task(forge_dir, &task)?;
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    let snippet = last_output.lines().next().unwrap_or("unknown error");
                    let snippet = if snippet.len() > 60 {
                        &snippet[..60]
                    } else {
                        snippet
                    };
                    println!(
                        "  {tag} {} {} failed: {} ({})",
                        "✗".red(),
                        task.id,
                        snippet.dimmed(),
                        format_duration(elapsed).dimmed()
                    );
                    break;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if attempt < MAX_RETRIES && is_transient_error(&err_str) {
                        let backoff = std::time::Duration::from_secs(10 * attempt as u64);
                        stats.retried.fetch_add(1, Ordering::Relaxed);
                        println!(
                            "  {tag} {} {} error, retry {}/{} in {}s...",
                            "↻".yellow(),
                            task.id,
                            attempt,
                            MAX_RETRIES,
                            backoff.as_secs()
                        );
                        std::thread::sleep(backoff);
                        continue;
                    }

                    let _guard = lock.lock().unwrap();
                    fail_task(forge_dir, &task)?;
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    println!(
                        "  {tag} {} {} error: {} ({})",
                        "✗".red(),
                        task.id,
                        e,
                        format_duration(elapsed).dimmed()
                    );
                    break;
                }
            }
        }

        if !success {
            // Log the failure event
            let event_logger = EventLogger::new(forge_dir);
            event_logger
                .log(
                    &ForgeEvent::new(
                        EventType::TaskCompleted, // reuse event type for now
                        format!(
                            "Task {} failed after {} attempts by {agent}",
                            task.id, attempt
                        ),
                    )
                    .with_task(&task.id)
                    .with_agent(agent.clone()),
                )
                .ok();
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
