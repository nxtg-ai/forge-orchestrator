use crate::brain::ForgeBrain;
use crate::brain::openai::OpenAIBrain;
use crate::brain::rule_based::RuleBasedBrain;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::plan::PlanManager;
use crate::core::state::StateManager;
use crate::core::task::{AgentType, TaskManager};
use colored::Colorize;
use std::path::Path;

pub fn execute(
    project_root: &Path,
    generate: bool,
    spec_path: Option<String>,
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

    if generate {
        generate_plan(project_root, &forge_dir, spec_path)?;
    } else {
        show_plan(&forge_dir)?;
    }

    Ok(())
}

fn show_plan(forge_dir: &Path) -> anyhow::Result<()> {
    let plan_mgr = PlanManager::new(forge_dir);

    if plan_mgr.has_plan() {
        let content = plan_mgr.read_plan()?;
        println!("{content}");
    } else {
        println!(
            "{} No plan found. Use {} to generate one from SPEC.md.",
            "!".yellow(),
            "forge plan --generate".cyan()
        );
    }

    Ok(())
}

fn generate_plan(
    project_root: &Path,
    forge_dir: &Path,
    spec_path: Option<String>,
) -> anyhow::Result<()> {
    println!("{}", "FORGE — CEO Mode: Plan Generation".bold());
    println!("{}", "=".repeat(40));
    println!();

    // Find spec file
    let spec_file = if let Some(path) = spec_path {
        project_root.join(path)
    } else {
        let candidates = ["SPEC.md", "spec.md", "PRD.md", "REQUIREMENTS.md"];
        let found = candidates
            .iter()
            .map(|f| project_root.join(f))
            .find(|p| p.exists());

        match found {
            Some(path) => path,
            None => {
                println!("{} No specification file found. Looked for:", "!".yellow());
                for c in &candidates {
                    println!("    - {c}");
                }
                println!();
                println!(
                    "Create a SPEC.md or use {} to specify a path.",
                    "--spec <path>".cyan()
                );
                return Ok(());
            }
        }
    };

    if !spec_file.exists() {
        println!("{} Spec file not found: {}", "✗".red(), spec_file.display());
        return Ok(());
    }

    println!("  {} Reading spec: {}", "→".cyan(), spec_file.display());
    let spec_content = std::fs::read_to_string(&spec_file)?;
    println!(
        "  {} Spec loaded ({} lines)",
        "✓".green(),
        spec_content.lines().count()
    );

    // Detect available tools
    let state_mgr = StateManager::new(forge_dir);
    let state = state_mgr.load()?;
    let available_tools: Vec<AgentType> = state
        .tools
        .iter()
        .filter(|t| t.available)
        .map(|t| t.agent_type.clone())
        .collect();

    println!(
        "  {} Available tools: {}",
        "→".cyan(),
        if available_tools.is_empty() {
            "none (using 'any')".to_string()
        } else {
            available_tools
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );

    // Select brain based on config
    let brain: Box<dyn ForgeBrain> = match state.brain.provider.as_str() {
        "openai" => {
            let model = state.brain.model.as_deref().unwrap_or("gpt-4.1");
            println!("  {} Using OpenAI brain (model: {model})", "→".cyan());
            Box::new(OpenAIBrain::new(model))
        }
        _ => {
            println!("  {} Using rule-based brain", "→".cyan());
            Box::new(RuleBasedBrain)
        }
    };

    // Use brain to decompose spec into tasks
    println!("  {} Decomposing spec into tasks...", "→".cyan());
    let tools_for_brain = if available_tools.is_empty() {
        vec![AgentType::Any]
    } else {
        available_tools.clone()
    };
    let mut tasks = brain.decompose_plan(&spec_content, &tools_for_brain)?;

    // Assign each task to an agent
    for task in &mut tasks {
        let assigned = brain.assign_task(task, &tools_for_brain)?;
        task.assigned_to = Some(assigned);
    }

    println!("  {} Generated {} tasks", "✓".green(), tasks.len());
    println!();

    // Display task table
    println!("{}", "Generated Plan:".bold());
    println!(
        "  {:<8} {:<40} {:<10} {:<10}",
        "ID".dimmed(),
        "Title".dimmed(),
        "Agent".dimmed(),
        "Status".dimmed()
    );
    println!("  {}", "-".repeat(68));

    for task in &tasks {
        let agent_str = task
            .assigned_to
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "any".into());
        println!(
            "  {:<8} {:<40} {:<10} {:<10}",
            task.id.cyan(),
            truncate(&task.title, 38),
            agent_str.yellow(),
            "pending".dimmed()
        );
    }
    println!();

    // Write tasks to .forge/tasks/
    let task_mgr = TaskManager::new(forge_dir);
    for task in &tasks {
        task_mgr.create_task(task)?;
    }
    println!("  {} Tasks written to .forge/tasks/", "✓".green());

    // Generate plan.md
    let plan_mgr = PlanManager::new(forge_dir);
    let plan_content = generate_plan_markdown(&state.project_name, &tasks);
    plan_mgr.write_plan(&plan_content)?;
    println!("  {} Plan written to .forge/plan.md", "✓".green());

    // Log event
    let event_logger = EventLogger::new(forge_dir);
    event_logger.log(&ForgeEvent::new(
        EventType::PlanCreated,
        format!(
            "Plan generated from {}: {} tasks",
            spec_file
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
            tasks.len()
        ),
    ))?;

    // Update state summary
    let summary = crate::core::state::TaskSummary {
        total: tasks.len(),
        pending: tasks.len(),
        ..Default::default()
    };
    state_mgr.update_task_summary(summary)?;

    println!();
    println!(
        "{} Plan ready. {} tasks across {} tools.",
        "✓".green().bold(),
        tasks.len().to_string().bold(),
        if available_tools.is_empty() {
            1
        } else {
            available_tools.len()
        }
    );
    println!();
    println!("Next steps:");
    println!("  {} — see the task board", "forge status".cyan());
    println!(
        "  {} — execute a task",
        "forge run --task T-001 --agent claude".cyan()
    );
    println!(
        "  {} — render config files for all tools",
        "forge sync".cyan()
    );

    Ok(())
}

fn generate_plan_markdown(project_name: &str, tasks: &[crate::core::task::Task]) -> String {
    let mut content = String::new();
    content.push_str(&format!("# {project_name} — Master Plan\n\n"));
    content.push_str("**Generated by:** Forge Orchestrator v0.1.0\n");
    content.push_str(&format!(
        "**Date:** {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    content.push_str(&format!("**Tasks:** {}\n\n", tasks.len()));
    content.push_str("---\n\n");
    content.push_str("## Task Board\n\n");
    content.push_str("| ID | Title | Agent | Status | Dependencies |\n");
    content.push_str("|----|-------|-------|--------|-------------|\n");

    for task in tasks {
        let agent = task
            .assigned_to
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "-".into());
        let deps = if task.depends_on.is_empty() {
            "-".to_string()
        } else {
            task.depends_on.join(", ")
        };
        content.push_str(&format!(
            "| {} | {} | {} | {:?} | {} |\n",
            task.id, task.title, agent, task.status, deps
        ));
    }

    content.push_str("\n---\n\n## Task Details\n\n");
    for task in tasks {
        content.push_str(&format!(
            "### {}: {}\n\n{}\n\n",
            task.id, task.title, task.description
        ));
    }

    content
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
