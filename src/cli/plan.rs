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

    // Resolve the input content: spec file or gathered project context
    let (spec_content, source_label) = resolve_plan_input(project_root, spec_path)?;

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
        "  {:<8} {:<36} {:<10} {:<12} {:<10}",
        "ID".dimmed(),
        "Title".dimmed(),
        "Agent".dimmed(),
        "Type".dimmed(),
        "Status".dimmed()
    );
    println!("  {}", "-".repeat(76));

    for task in &tasks {
        let agent_str = task
            .assigned_to
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "any".into());
        let type_str = task.task_type.as_deref().unwrap_or("—");
        println!(
            "  {:<8} {:<36} {:<10} {:<12} {:<10}",
            task.id.cyan(),
            truncate(&task.title, 34),
            agent_str.yellow(),
            type_str.dimmed(),
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
        format!("Plan generated from {source_label}: {} tasks", tasks.len()),
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
    content.push_str("| ID | Title | Agent | Type | Status | Dependencies |\n");
    content.push_str("|----|-------|-------|------|--------|-------------|\n");

    for task in tasks {
        let agent = task
            .assigned_to
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "-".into());
        let task_type = task.task_type.as_deref().unwrap_or("-");
        let deps = if task.depends_on.is_empty() {
            "-".to_string()
        } else {
            task.depends_on.join(", ")
        };
        content.push_str(&format!(
            "| {} | {} | {} | {} | {:?} | {} |\n",
            task.id, task.title, agent, task_type, task.status, deps
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

/// Resolve plan input: either a spec file or gathered project context.
/// Returns (content, source_label) where source_label describes where the content came from.
fn resolve_plan_input(
    project_root: &Path,
    spec_path: Option<String>,
) -> anyhow::Result<(String, String)> {
    // If explicit --spec path given, use that
    if let Some(path) = spec_path {
        let spec_file = project_root.join(&path);
        if !spec_file.exists() {
            anyhow::bail!("Spec file not found: {}", spec_file.display());
        }
        let content = std::fs::read_to_string(&spec_file)?;
        println!("  {} Reading spec: {}", "→".cyan(), spec_file.display());
        println!(
            "  {} Spec loaded ({} lines)",
            "✓".green(),
            content.lines().count()
        );
        let label = spec_file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        return Ok((content, label));
    }

    // Try to find a spec file automatically
    let exact_candidates = ["SPEC.md", "spec.md", "PRD.md", "REQUIREMENTS.md"];
    let found = exact_candidates
        .iter()
        .map(|f| project_root.join(f))
        .find(|p| p.exists())
        .or_else(|| find_spec_by_glob(project_root));

    if let Some(spec_file) = found {
        let content = std::fs::read_to_string(&spec_file)?;
        println!("  {} Reading spec: {}", "→".cyan(), spec_file.display());
        println!(
            "  {} Spec loaded ({} lines)",
            "✓".green(),
            content.lines().count()
        );
        let label = spec_file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        return Ok((content, label));
    }

    // No spec file — gather project context instead
    println!(
        "  {} No SPEC.md found — gathering project context...",
        "→".cyan()
    );
    let context = gather_project_context(project_root)?;
    if context.is_empty() {
        anyhow::bail!(
            "No project context found. Create a README.md or SPEC.md, then run `forge plan --generate`."
        );
    }
    println!(
        "  {} Project context gathered ({} lines)",
        "✓".green(),
        context.lines().count()
    );
    Ok((context, "project context".to_string()))
}

/// Gather project context from README, markdown files, and manifest files.
/// This is used when no SPEC.md exists — the brain analyzes whatever context is available.
fn gather_project_context(project_root: &Path) -> anyhow::Result<String> {
    let mut sections: Vec<String> = Vec::new();

    // 1. README.md — primary context
    let readme = project_root.join("README.md");
    if readme.exists() {
        let content = std::fs::read_to_string(&readme)?;
        if !content.trim().is_empty() {
            println!("    {} README.md", "✓".green());
            sections.push(format!("# README.md\n\n{content}"));
        }
    }

    // 2. Project manifest — tech stack detection
    for (name, label) in [
        ("package.json", "Node.js manifest"),
        ("Cargo.toml", "Rust manifest"),
        ("pyproject.toml", "Python manifest"),
        ("go.mod", "Go manifest"),
    ] {
        let path = project_root.join(name);
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let truncated: String = content.lines().take(80).collect::<Vec<_>>().join("\n");
            println!("    {} {} ({})", "✓".green(), name, label);
            sections.push(format!("# {name} ({label})\n\n```\n{truncated}\n```"));
        }
    }

    // 3. Other markdown files in root — CLAUDE.md, AGENTS.md, CONTRIBUTING.md, etc.
    if let Ok(entries) = std::fs::read_dir(project_root) {
        let mut md_files: Vec<_> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.ends_with(".md")
                    && name != "README.md"
                    && !name.starts_with("CHANGELOG")
                    && !name.starts_with("LICENSE")
            })
            .collect();
        md_files.sort_by_key(|e| e.file_name());

        for entry in md_files {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(content) = std::fs::read_to_string(entry.path())
                && !content.trim().is_empty()
            {
                let truncated: String =
                    content.lines().take(100).collect::<Vec<_>>().join("\n");
                println!("    {} {}", "✓".green(), name);
                sections.push(format!("# {name}\n\n{truncated}"));
            }
        }
    }

    // 4. Markdown files in docs/ directory
    let docs_dir = project_root.join("docs");
    if docs_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&docs_dir)
    {
        let mut doc_files: Vec<_> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
            .collect();
        doc_files.sort_by_key(|e| e.file_name());

        for entry in doc_files.into_iter().take(5) {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(content) = std::fs::read_to_string(entry.path())
                && !content.trim().is_empty()
            {
                let truncated: String =
                    content.lines().take(80).collect::<Vec<_>>().join("\n");
                println!("    {} docs/{}", "✓".green(), name);
                sections.push(format!("# docs/{name}\n\n{truncated}"));
            }
        }
    }

    Ok(sections.join("\n\n---\n\n"))
}

fn find_spec_by_glob(project_root: &Path) -> Option<std::path::PathBuf> {
    let prefixes = ["SPEC-", "spec-", "PRD-", "REQUIREMENTS-"];
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") && prefixes.iter().any(|p| name.starts_with(p)) {
                return Some(entry.path());
            }
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
