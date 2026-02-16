use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::adapters::{ToolAdapter, execute_command_async};
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskStatus};
use crate::detect;
use colored::Colorize;
use rand::Rng;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

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
    "500",
    "502",
    "503",
    "overloaded",
    "api_error",
];

/// Rate-limit-specific error signatures (subset of transient errors).
const RATE_LIMIT_ERRORS: &[&str] = &[
    "usage_limit_reached",
    "rate_limit",
    "rate limit",
    "too many requests",
    "no capacity available",
    "overloaded",
    "rate_limit_error",
];

/// Max consecutive rate limits before pausing a provider for a long time.
const MAX_CONSECUTIVE_RATE_LIMITS: u32 = 3;

/// Shared orchestration stats.
struct OrcStats {
    completed: AtomicUsize,
    failed: AtomicUsize,
    retried: AtomicUsize,
    total: usize,
}

/// Per-provider rate limit tracking (in-memory, not persisted).
#[derive(Debug, Default)]
struct ProviderState {
    consecutive_rate_limits: u32,
    paused_until: Option<Instant>,
}

type ProviderTracker = Arc<Mutex<HashMap<AgentType, ProviderState>>>;

/// CEO Mode: loop `execute()` until all tasks are done or permanently stuck.
/// Resets failed tasks between passes (treats them as retryable).
pub async fn execute_loop(project_root: &Path, agent_filter: Option<&str>) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    let mut pass = 0u32;
    const MAX_PASSES: u32 = 5;

    loop {
        pass += 1;
        if pass > 1 {
            println!();
            println!(
                "  {} CEO Mode — Pass {}/{}",
                "♻".cyan().bold(),
                pass,
                MAX_PASSES
            );
            println!();
        }

        execute(project_root, agent_filter).await?;

        // Check remaining work
        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks()?;
        let pending = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending || t.status == TaskStatus::Blocked)
            .count();
        let failed = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Failed)
            .count();
        let total_remaining = pending + failed;

        if total_remaining == 0 {
            println!(
                "\n  {} CEO Mode: All tasks complete. Ship it.",
                "✓".green().bold()
            );
            break;
        }

        if pass >= MAX_PASSES {
            println!(
                "\n  {} CEO Mode: {} tasks remain after {} passes. Manual review needed.",
                "!".yellow(),
                total_remaining,
                MAX_PASSES
            );
            break;
        }

        // Reset failed tasks for retry in next pass
        if failed > 0 {
            println!(
                "\n  {} Resetting {} failed tasks for retry...",
                "↻".yellow(),
                failed
            );
            for task in &tasks {
                if task.status == TaskStatus::Failed {
                    let mut updated = task.clone();
                    updated.status = TaskStatus::Pending;
                    updated.updated_at = chrono::Utc::now();
                    task_mgr.update_task(&updated)?;
                }
            }
        }

        // Wait before next pass
        println!("\n  {} Waiting 30s before next pass...\n", "⏳".dimmed());
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }

    Ok(())
}

/// Autonomous orchestration — runs all tasks respecting dependencies,
/// one task per agent type, with retry, progress tracking, and auto-sync.
pub async fn execute(project_root: &Path, agent_filter: Option<&str>) -> anyhow::Result<()> {
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
        "╔══════════════════════════════════════════════╗".bright_cyan()
    );
    println!(
        "  {}  {}  {}",
        "║".bright_cyan(),
        "FORGE — Autonomous Orchestration".bold().white(),
        "       ║".bright_cyan()
    );
    println!(
        "  {}",
        "╚══════════════════════════════════════════════╝".bright_cyan()
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

    // ── Load scheduler config ────────────────────────────────────
    let state_mgr = StateManager::new(&forge_dir);
    let state = state_mgr.load()?;
    let scheduler = state.scheduler.clone();

    // Show pacing info
    for agent in &target_agents {
        let agent_name = agent.to_string().to_lowercase();
        let auth = state
            .agent_auth
            .get(&agent_name)
            .cloned()
            .unwrap_or_else(|| "subscription".to_string());
        if auth == "subscription" {
            println!(
                "  {} {:<8} subscription mode — pacing {}-{}s between tasks",
                "⏱".dimmed(),
                agent_name,
                scheduler.pacing_min_secs,
                scheduler.pacing_max_secs,
            );
        } else {
            println!(
                "  {} {:<8} API mode — full speed",
                "⚡".dimmed(),
                agent_name,
            );
        }
    }
    if scheduler.rotation {
        println!(
            "  {} Provider rotation: {}",
            "↻".dimmed(),
            "enabled".green()
        );
    }
    println!();

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
            .filter(|t| t.assigned_to.as_ref() == Some(agent) && t.status == TaskStatus::Completed)
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
        println!("\n  {} All tasks already complete!", "✓".green().bold());
        return Ok(());
    }

    // ── Shared stats ────────────────────────────────────────────
    let stats = Arc::new(OrcStats {
        completed: AtomicUsize::new(already_done),
        failed: AtomicUsize::new(0),
        retried: AtomicUsize::new(0),
        total,
    });

    // ── Provider tracker (rate limit state) ──────────────────────
    let provider_tracker: ProviderTracker = Arc::new(Mutex::new(HashMap::new()));

    // ── Run orchestration ───────────────────────────────────────
    let lock = Arc::new(Mutex::new(()));
    let scheduler = Arc::new(scheduler);

    if target_agents.len() == 1 {
        run_agent_loop(
            project_root,
            &forge_dir,
            &target_agents[0],
            &lock,
            &stats,
            &provider_tracker,
            &scheduler,
        )
        .await?;
    } else {
        run_parallel(
            project_root,
            &forge_dir,
            &target_agents,
            &stats,
            &provider_tracker,
            &scheduler,
        )
        .await?;
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
        "╔══════════════════════════════════════════════╗".bright_cyan()
    );
    println!(
        "  {}  {}  {}",
        "║".bright_cyan(),
        "Orchestration Complete".bold().white(),
        "                ║".bright_cyan()
    );
    println!(
        "  {}",
        "╚══════════════════════════════════════════════╝".bright_cyan()
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
            .filter(|t| t.assigned_to.as_ref() == Some(agent) && t.status == TaskStatus::Completed)
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
    println!("  [{}] {}/{} ({:.0}%)", bar, done, total, pct * 100.0);
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

/// DX-034: Detect if an error is specifically a rate limit (not just any transient error).
fn is_rate_limit_error(output: &str) -> bool {
    let lower = output.to_lowercase();
    RATE_LIMIT_ERRORS.iter().any(|e| lower.contains(e))
        || (lower.contains("429")
            && (lower.contains("capacity") || lower.contains("limit") || lower.contains("quota")))
}

/// Run multiple agents in parallel, each managing its own task queue.
async fn run_parallel(
    project_root: &Path,
    forge_dir: &Path,
    agents: &[AgentType],
    stats: &Arc<OrcStats>,
    provider_tracker: &ProviderTracker,
    scheduler: &Arc<crate::core::state::SchedulerConfig>,
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
            let tracker = Arc::clone(provider_tracker);
            let sched = Arc::clone(scheduler);

            tokio::spawn(async move {
                run_agent_loop(&pr, &fd, &agent, &lock, &stats, &tracker, &sched).await
            })
        })
        .collect();

    let mut errors = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(e) => errors.push(format!("Agent task panicked: {e}")),
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

/// Agent loop with pacing, rate limit detection, backoff, and rotation.
async fn run_agent_loop(
    project_root: &Path,
    forge_dir: &Path,
    agent: &AgentType,
    lock: &Arc<Mutex<()>>,
    stats: &Arc<OrcStats>,
    provider_tracker: &ProviderTracker,
    scheduler: &Arc<crate::core::state::SchedulerConfig>,
) -> anyhow::Result<()> {
    let tag = format!("[{}]", agent).cyan();
    let mut consecutive_waits = 0u32;

    // DX-033: Get auth mode for this agent (determines pacing)
    let auth_mode = {
        let state_mgr = StateManager::new(forge_dir);
        let agent_name = agent.to_string().to_lowercase();
        state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string())
    };

    loop {
        // DX-035: Check if this provider is paused (rate limited)
        {
            let tracker = provider_tracker.lock().await;
            if let Some(state) = tracker.get(agent)
                && let Some(paused_until) = state.paused_until
                && Instant::now() < paused_until
            {
                let remaining = paused_until - Instant::now();
                println!(
                    "  {tag} {} Provider rate-limited, resuming in {}s...",
                    "⏸".yellow(),
                    remaining.as_secs()
                );
                drop(tracker);
                tokio::time::sleep(remaining).await;
                continue;
            }
        }

        // Under lock: find next available task for this agent
        let next_task = {
            let _guard = lock.lock().await;
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
                            && (t.status == TaskStatus::Pending || t.status == TaskStatus::Blocked)
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
                let current_done = stats.completed.load(Ordering::Relaxed);
                let current_failed = stats.failed.load(Ordering::Relaxed);
                let total_finished = current_done + current_failed;

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

                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }
        };

        // Claim the task (under lock)
        {
            let _guard = lock.lock().await;
            claim_task(forge_dir, &task, agent)?;
        }

        println!("  {tag} {} {} {}", "→".cyan(), task.id.bold(), task.title);

        // ── Execute with retry ──────────────────────────────────
        let started = Instant::now();
        let mut attempt = 0;
        let mut last_output;
        let mut success = false;
        let mut hit_rate_limit = false;

        loop {
            attempt += 1;
            let result = execute_task(&task, agent, project_root).await;
            let elapsed = started.elapsed();

            match result {
                Ok(output) if output.success => {
                    let _guard = lock.lock().await;
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

                    // Reset consecutive rate limits on success
                    {
                        let mut tracker = provider_tracker.lock().await;
                        if let Some(state) = tracker.get_mut(agent) {
                            state.consecutive_rate_limits = 0;
                            state.paused_until = None;
                        }
                    }

                    break;
                }
                Ok(output) => {
                    last_output = output.output.clone();
                    save_result(forge_dir, &task.id, &last_output).ok();

                    // DX-034: Check if this is specifically a rate limit error
                    if is_rate_limit_error(&last_output) {
                        hit_rate_limit = true;
                        break;
                    }

                    // Check if retryable (non-rate-limit transient error)
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
                            let _guard = lock.lock().await;
                            let task_mgr = TaskManager::new(forge_dir);
                            let mut updated = task.clone();
                            updated.status = TaskStatus::InProgress;
                            updated.updated_at = chrono::Utc::now();
                            task_mgr.update_task(&updated)?;
                        }

                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    // Permanent failure
                    let _guard = lock.lock().await;
                    fail_task(forge_dir, &task)?;
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    let snippet = last_output.lines().next().unwrap_or("unknown error");
                    let snippet = truncate_safe(snippet, 60);
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

                    // Check for rate limit in error
                    if is_rate_limit_error(&err_str) {
                        last_output = err_str;
                        save_result(forge_dir, &task.id, &last_output).ok();
                        hit_rate_limit = true;
                        break;
                    }

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
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    let _guard = lock.lock().await;
                    fail_task(forge_dir, &task)?;
                    stats.failed.fetch_add(1, Ordering::Relaxed);
                    println!(
                        "  {tag} {} {} error: {} ({})",
                        "✗".red(),
                        task.id,
                        e,
                        format_duration(started.elapsed()).dimmed()
                    );
                    break;
                }
            }
        }

        // DX-034/035: Handle rate limit — DON'T fail the task, pause the provider
        if hit_rate_limit {
            let mut tracker = provider_tracker.lock().await;
            let pstate = tracker.entry(agent.clone()).or_default();
            pstate.consecutive_rate_limits += 1;

            // DX-035: Exponential backoff — 60s, 120s, 240s, 480s, 600s max
            let backoff_secs = std::cmp::min(
                60 * 2u64.pow(pstate.consecutive_rate_limits.saturating_sub(1)),
                600,
            );

            // Reset task to pending so it can be retried later
            {
                let _guard = lock.lock().await;
                let task_mgr = TaskManager::new(forge_dir);
                let mut updated = task.clone();
                updated.status = TaskStatus::Pending;
                updated.updated_at = chrono::Utc::now();
                task_mgr.update_task(&updated)?;
            }

            if pstate.consecutive_rate_limits >= MAX_CONSECUTIVE_RATE_LIMITS {
                // Long pause — 30 minutes
                pstate.paused_until = Some(Instant::now() + Duration::from_secs(1800));
                println!(
                    "  {tag} {} {} rate limited ({} consecutive). Pausing provider for 30 minutes.",
                    "⛔".red(),
                    task.id,
                    pstate.consecutive_rate_limits,
                );

                // DX-036: Provider rotation — reassign remaining tasks if enabled
                if scheduler.rotation {
                    drop(tracker);
                    let reassigned = rotate_tasks(forge_dir, agent, provider_tracker).await;
                    if reassigned > 0 {
                        println!(
                            "  {tag} {} Rotated {} tasks to available providers.",
                            "↻".cyan(),
                            reassigned,
                        );
                    } else {
                        println!(
                            "  {tag} {} No available providers for rotation.",
                            "!".yellow(),
                        );
                    }
                } else {
                    drop(tracker);
                    println!(
                        "  {tag} {} Enable rotation: forge config scheduler.rotation enabled",
                        "💡".dimmed(),
                    );
                }
            } else {
                pstate.paused_until = Some(Instant::now() + Duration::from_secs(backoff_secs));
                println!(
                    "  {tag} {} {} rate limited. Pausing provider for {}s (attempt {}/{}).",
                    "⏸".yellow(),
                    task.id,
                    backoff_secs,
                    pstate.consecutive_rate_limits,
                    MAX_CONSECUTIVE_RATE_LIMITS,
                );
                drop(tracker);
            }

            // Don't immediately retry — go back to top of loop where pause check happens
            continue;
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

        // DX-033: Subscription pacing — random delay between tasks
        if success && auth_mode == "subscription" {
            let delay =
                rand::rng().random_range(scheduler.pacing_min_secs..=scheduler.pacing_max_secs);
            println!(
                "  {tag} {} Subscription pacing: waiting {}s before next task...",
                "⏳".dimmed(),
                delay,
            );
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }

    Ok(())
}

/// DX-036: Rotate pending tasks from a paused provider to available providers.
/// Returns the number of tasks reassigned.
async fn rotate_tasks(
    forge_dir: &Path,
    paused_agent: &AgentType,
    provider_tracker: &ProviderTracker,
) -> usize {
    let task_mgr = TaskManager::new(forge_dir);
    let tasks = match task_mgr.list_tasks() {
        Ok(t) => t,
        Err(_) => return 0,
    };

    // Find pending tasks assigned to the paused provider
    let pending_for_paused: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending && t.assigned_to.as_ref() == Some(paused_agent))
        .collect();

    if pending_for_paused.is_empty() {
        return 0;
    }

    // Find available providers (not paused)
    let tracker = provider_tracker.lock().await;
    let all_agents = [AgentType::Claude, AgentType::Codex, AgentType::Gemini];
    let available: Vec<_> = all_agents
        .iter()
        .filter(|a| {
            *a != paused_agent
                && tracker
                    .get(a)
                    .and_then(|s| s.paused_until)
                    .is_none_or(|until| Instant::now() >= until)
        })
        .collect();
    drop(tracker);

    if available.is_empty() {
        return 0;
    }

    // Round-robin reassignment
    let mut reassigned = 0;
    for (i, task) in pending_for_paused.iter().enumerate() {
        let target = available[i % available.len()];
        let mut updated = (*task).clone();
        updated.assigned_to = Some(target.clone());
        updated.updated_at = chrono::Utc::now();
        if task_mgr.update_task(&updated).is_ok() {
            reassigned += 1;
        }
    }

    reassigned
}

/// UTF-8 safe truncation.
fn truncate_safe(s: &str, max: usize) -> &str {
    if s.chars().count() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        &s[..end]
    }
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

/// Execute a task via the appropriate CLI adapter (async).
async fn execute_task(
    task: &Task,
    agent: &AgentType,
    project_root: &Path,
) -> anyhow::Result<crate::adapters::ExecutionResult> {
    // Read per-agent config from state
    let forge_dir = project_root.join(".forge");
    let agent_name = agent.to_string().to_lowercase();
    let (auth_mode, permissions) = if forge_dir.exists() {
        let state_mgr = crate::core::state::StateManager::new(&forge_dir);
        let auth = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());
        let perms = state_mgr
            .get_agent_permissions(&agent_name)
            .unwrap_or_else(|_| "safe".to_string());
        (auth, perms)
    } else {
        ("subscription".to_string(), "safe".to_string())
    };

    let cmd = match agent {
        AgentType::Claude => {
            ClaudeAdapter.build_command(task, project_root, &auth_mode, &permissions)
        }
        AgentType::Codex => {
            CodexAdapter.build_command(task, project_root, &auth_mode, &permissions)
        }
        AgentType::Gemini => {
            GeminiAdapter.build_command(task, project_root, &auth_mode, &permissions)
        }
        AgentType::Any => ClaudeAdapter.build_command(task, project_root, &auth_mode, &permissions),
    };

    execute_command_async(cmd).await
}

/// DX-038: Check if Claude is in subscription mode and return warning text if so.
/// Returns `Some(warning_string)` if risk is detected, `None` if safe.
pub fn load_state_for_risk_check(forge_dir: &Path) -> Option<String> {
    let state_mgr = StateManager::new(forge_dir);
    let state = state_mgr.load().ok()?;
    let has_claude_sub = state
        .agent_auth
        .get("claude")
        .map(|v| v == "subscription")
        .unwrap_or(true);
    if has_claude_sub {
        Some(
            "\n\
             \x20 \u{26A0}\u{26A0}\u{26A0}  SUBSCRIPTION RISK DETECTED  \u{26A0}\u{26A0}\u{26A0}\n\
             \n\
             \x20 Claude is configured to use subscription auth.\n\
             \x20 Anthropic ACTIVELY BLOCKS third-party CLI orchestration\n\
             \x20 and has BANNED accounts for this pattern.\n\
             \n\
             \x20 Options:\n\
             \x20   1. Switch to API mode (RECOMMENDED):\n\
             \x20      forge config claude.auth api\n\
             \n\
             \x20   2. Accept the risk:\n\
             \x20      forge start --i-accept-subscription-risk\n\
             \n\
             \x20 See: docs/research/cli-subscription-gating-analysis.md"
                .to_string(),
        )
    } else {
        None
    }
}
