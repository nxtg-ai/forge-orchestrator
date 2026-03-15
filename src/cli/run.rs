use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::adapters::{ToolAdapter, execute_command_async};
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskStatus};
use crate::tui::app::commit_type_for_task;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("  {spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&[
            "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
            "\u{2827}", "\u{2807}", "\u{280f}", " ",
        ])
}

pub async fn execute(project_root: &Path, task_id: &str, agent_name: &str) -> anyhow::Result<()> {
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

    // Read per-agent config (defaults: subscription auth, safe permissions)
    let auth_mode = state_mgr
        .get_agent_auth(agent_name)
        .unwrap_or_else(|_| "subscription".to_string());
    let permissions = state_mgr
        .get_agent_permissions(agent_name)
        .unwrap_or_else(|_| "safe".to_string());

    // Build command via adapter, execute asynchronously with spinner
    let sp = ProgressBar::new_spinner();
    sp.set_style(spinner_style());
    sp.set_message(format!("Running {} on {}...", agent_name, task_id));
    sp.enable_steady_tick(std::time::Duration::from_millis(80));

    let cmd = build_adapter_command(&task, project_root, &agent_type, &auth_mode, &permissions);
    let result = execute_command_async(cmd).await?;

    // Save result
    let results_dir = forge_dir.join("results");
    std::fs::create_dir_all(&results_dir)?;
    std::fs::write(results_dir.join(format!("{task_id}.txt")), &result.output)?;

    // Update task status based on result
    if result.success {
        task.status = TaskStatus::Completed;
        task.completed_at = Some(chrono::Utc::now());
        sp.finish_and_clear();
        println!("  {} {} completed", "✓".green(), task_id);

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
        sp.finish_and_clear();
        println!(
            "  {} {} failed (exit code: {})",
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

/// Build the adapter command for a given agent type.
fn build_adapter_command(
    task: &Task,
    project_root: &Path,
    agent: &AgentType,
    auth_mode: &str,
    permissions: &str,
) -> std::process::Command {
    match agent {
        AgentType::Claude | AgentType::Any => {
            ClaudeAdapter.build_command(task, project_root, auth_mode, permissions)
        }
        AgentType::Codex => CodexAdapter.build_command(task, project_root, auth_mode, permissions),
        AgentType::Gemini => {
            GeminiAdapter.build_command(task, project_root, auth_mode, permissions)
        }
    }
}

/// Run git auto-commit for a completed task (headless mode).
fn git_auto_commit_headless(forge_dir: &Path, project_root: &Path, task: &Task, agent: &AgentType) {
    let state_mgr = StateManager::new(forge_dir);
    let enabled = state_mgr.load().map(|s| s.git.auto_commit).unwrap_or(true);

    if !enabled || !project_root.join(".git").exists() {
        return;
    }

    let commit_type = commit_type_for_task(task.task_type.as_deref());
    let message = format!("{}({}): {}", commit_type, task.id, task.title);

    println!(
        "  {} auto-commit: {}({}) by {}",
        "⟳".dimmed(),
        commit_type,
        task.id,
        agent
    );

    let add_result = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(project_root)
        .output();

    if let Err(e) = add_result {
        eprintln!("  git add failed for {}: {}", task.id, e);
        return;
    }

    let commit_result = std::process::Command::new("git")
        .args(["commit", "-m", &message, "--allow-empty=false"])
        .current_dir(project_root)
        .output();

    match commit_result {
        Ok(output) if output.status.success() => {
            println!("  {} committed: {}", "⟳".green(), message.dimmed());
        }
        Ok(_) => {} // Nothing to commit (clean tree)
        Err(e) => eprintln!("  git commit failed for {}: {}", task.id, e),
    }
}

/// Autonomous headless execution: run all tasks in parallel with dependency awareness.
pub async fn execute_all(
    project_root: &Path,
    parallel_limit: usize,
    dry_run: bool,
) -> anyhow::Result<()> {
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

    let tasks = task_mgr.list_tasks()?;
    if tasks.is_empty() {
        println!(
            "{} No tasks found. Run {} first.",
            "!".yellow(),
            "forge plan --generate".cyan()
        );
        return Ok(());
    }

    // Reset orphaned in-progress tasks (same as dashboard DX-021)
    for task in &tasks {
        if matches!(task.status, TaskStatus::InProgress | TaskStatus::Assigned) {
            let mut updated = task.clone();
            updated.status = TaskStatus::Pending;
            updated.updated_at = chrono::Utc::now();
            task_mgr.update_task(&updated)?;
        }
    }

    let total = tasks.len();
    let completed_count = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();

    println!();
    println!("{}", "FORGE — Autonomous Execution".bold());
    println!("{}", "=".repeat(40));
    println!(
        "  Tasks: {} total, {} already done, {} to run",
        total,
        completed_count,
        total - completed_count
    );
    println!("  Parallel: {}", parallel_limit);
    println!();

    if dry_run {
        println!("{}", "Dry run — showing execution plan:".yellow());
        let completed_ids: Vec<String> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        for task in &tasks {
            if task.status == TaskStatus::Completed {
                println!("  {} {} (already done)", "✓".green(), task.id);
                continue;
            }
            let agent = task
                .assigned_to
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "auto".into());
            if task.is_blocked(&completed_ids) {
                let pending_deps: Vec<_> = task
                    .depends_on
                    .iter()
                    .filter(|d| !completed_ids.contains(d))
                    .map(|d| d.as_str())
                    .collect();
                println!(
                    "  {} {} [{}] — blocked by {}",
                    "⏳".yellow(),
                    task.id,
                    agent,
                    pending_deps.join(", ")
                );
            } else {
                println!("  {} {} [{}] — ready", "○".white(), task.id, agent);
            }
        }
        println!();
        println!("Run without {} to execute.", "--dry-run".cyan());
        return Ok(());
    }

    // Main loop: keep going until all done or stuck
    let start_time = Instant::now();
    let mut running: HashMap<String, tokio::task::JoinHandle<(String, AgentType, bool, i32)>> =
        HashMap::new();

    loop {
        let tasks = task_mgr.list_tasks()?;
        let completed_ids: Vec<String> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        // Check if all done
        let all_done = tasks
            .iter()
            .all(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Failed));
        if all_done && running.is_empty() {
            break;
        }

        // Find unblocked pending tasks
        let ready: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending && !t.is_blocked(&completed_ids))
            .collect();

        // Spawn up to parallel_limit
        let slots = parallel_limit.saturating_sub(running.len());
        for task in ready.into_iter().take(slots) {
            if running.contains_key(&task.id) {
                continue;
            }

            let agent = task.assigned_to.clone().unwrap_or(AgentType::Claude);
            let agent = if agent == AgentType::Any {
                AgentType::Claude
            } else {
                agent
            };

            // Mark as in-progress
            let mut updated = task.clone();
            updated.status = TaskStatus::InProgress;
            updated.assigned_to = Some(agent.clone());
            updated.updated_at = chrono::Utc::now();
            task_mgr.update_task(&updated)?;

            println!("  {} {} started on {}", "▶".cyan(), task.id, agent);

            let task_clone = task.clone();
            let project_root = project_root.to_path_buf();
            let forge_dir_clone = forge_dir.clone();
            let agent_clone = agent.clone();

            let handle = tokio::spawn(async move {
                let sm = StateManager::new(&forge_dir_clone);
                let auth_mode = sm
                    .get_agent_auth(&agent_clone.to_string())
                    .unwrap_or_else(|_| "subscription".to_string());
                let permissions = sm
                    .get_agent_permissions(&agent_clone.to_string())
                    .unwrap_or_else(|_| "safe".to_string());

                let cmd = build_adapter_command(
                    &task_clone,
                    &project_root,
                    &agent_clone,
                    &auth_mode,
                    &permissions,
                );
                let result = execute_command_async(cmd).await;

                match result {
                    Ok(r) => (task_clone.id.clone(), agent_clone, r.success, r.exit_code),
                    Err(_) => (task_clone.id.clone(), agent_clone, false, -1),
                }
            });

            running.insert(task.id.clone(), handle);
        }

        // Check for completed handles
        let mut finished = Vec::new();
        for (task_id, handle) in &running {
            if handle.is_finished() {
                finished.push(task_id.clone());
            }
        }

        for task_id in finished {
            if let Some(handle) = running.remove(&task_id) {
                match handle.await {
                    Ok((tid, agent, success, exit_code)) => {
                        if let Ok(task) = task_mgr.get_task(&tid) {
                            let mut updated = task;
                            if success {
                                updated.status = TaskStatus::Completed;
                                updated.completed_at = Some(chrono::Utc::now());
                                println!(
                                    "  {} {} completed (exit {})",
                                    "✓".green(),
                                    tid,
                                    exit_code
                                );
                                git_auto_commit_headless(
                                    &forge_dir,
                                    project_root,
                                    &updated,
                                    &agent,
                                );
                            } else {
                                updated.status = TaskStatus::Failed;
                                println!("  {} {} failed (exit {})", "✗".red(), tid, exit_code);
                            }
                            updated.updated_at = chrono::Utc::now();
                            task_mgr.update_task(&updated).ok();
                            state_mgr.unlock_files(&tid).ok();
                        }
                    }
                    Err(e) => {
                        println!("  {} {} panicked: {}", "✗".red(), task_id, e);
                    }
                }
            }
        }

        // If nothing is running and nothing is ready, we're stuck
        if running.is_empty() {
            let tasks = task_mgr.list_tasks()?;
            let pending: Vec<_> = tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Pending)
                .collect();
            if !pending.is_empty() {
                println!();
                println!(
                    "  {} {} tasks blocked — dependencies failed or circular",
                    "!".yellow(),
                    pending.len()
                );
                for t in &pending {
                    println!(
                        "    {} blocked by: {}",
                        t.id.dimmed(),
                        t.depends_on.join(", ")
                    );
                }
                break;
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Summary
    let elapsed = start_time.elapsed();
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
    println!("{}", "─".repeat(40));
    println!(
        "  {} {}/{} completed, {} failed in {:.0}s",
        if failed == 0 {
            "✓".green()
        } else {
            "!".yellow()
        },
        completed,
        total,
        failed,
        elapsed.as_secs_f64()
    );
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_adapter_command_claude() {
        let task = make_test_task("T-001");
        let cmd = build_adapter_command(
            &task,
            Path::new("/tmp"),
            &AgentType::Claude,
            "subscription",
            "safe",
        );
        let prog = cmd.get_program().to_str().unwrap_or("");
        assert!(
            prog.contains("claude"),
            "expected claude in program: {prog}"
        );
    }

    #[test]
    fn test_build_adapter_command_any_defaults_to_claude() {
        let task = make_test_task("T-001");
        let cmd_any = build_adapter_command(
            &task,
            Path::new("/tmp"),
            &AgentType::Any,
            "subscription",
            "safe",
        );
        let cmd_claude = build_adapter_command(
            &task,
            Path::new("/tmp"),
            &AgentType::Claude,
            "subscription",
            "safe",
        );
        assert_eq!(
            cmd_any.get_program().to_str(),
            cmd_claude.get_program().to_str()
        );
    }

    #[test]
    fn test_build_adapter_command_codex() {
        let task = make_test_task("T-001");
        let cmd = build_adapter_command(&task, Path::new("/tmp"), &AgentType::Codex, "api", "yolo");
        let prog = cmd.get_program().to_str().unwrap_or("");
        assert!(prog.contains("codex"), "expected codex in program: {prog}");
    }

    #[test]
    fn test_build_adapter_command_gemini() {
        let task = make_test_task("T-001");
        let cmd = build_adapter_command(
            &task,
            Path::new("/tmp"),
            &AgentType::Gemini,
            "subscription",
            "safe",
        );
        let prog = cmd.get_program().to_str().unwrap_or("");
        assert!(
            prog.contains("gemini"),
            "expected gemini in program: {prog}"
        );
    }

    fn make_test_task(id: &str) -> Task {
        Task {
            id: id.to_string(),
            title: format!("Test task {id}"),
            description: "test".to_string(),
            status: TaskStatus::Pending,
            assigned_to: None,
            task_type: Some("implement".to_string()),
            depends_on: vec![],
            locked_files: vec![],
            acceptance_criteria: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
            plan_version: None,
            parent_task: None,
            phase: None,
            retry_count: 0,
            verified_by: None,
            verified_at: None,
        }
    }
}
