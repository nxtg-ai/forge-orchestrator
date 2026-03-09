use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::plan::PlanManager;
use crate::core::state::{ForgeState, StateManager};
use crate::detect::{detect_tools, display_detected_tools};
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, name: Option<String>) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    // Check if already initialized
    if forge_dir.exists() {
        println!(
            "{} Forge is already initialized in this project.",
            "!".yellow()
        );
        println!("  Use {} to see current state.", "forge status".cyan());
        return Ok(());
    }

    println!("{}", "FORGE ORCHESTRATOR — Initialization".bold());
    println!("{}", "=".repeat(40));
    println!();

    // Step 0: Context Discovery
    println!("{}", "Step 1: Context Discovery".bold());
    println!("Scanning project for existing context...\n");

    let project_name = name.unwrap_or_else(|| {
        project_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed-project".into())
    });

    // Scan for project files
    let package_json_exists = project_root.join("package.json").exists();
    let cargo_toml_exists = project_root.join("Cargo.toml").exists();
    let git_exists = project_root.join(".git").exists();
    let pyproject_toml_exists = project_root.join("pyproject.toml").exists();
    let requirements_txt_exists = project_root.join("requirements.txt").exists();
    let setup_py_exists = project_root.join("setup.py").exists();
    let pipfile_exists = project_root.join("Pipfile").exists();
    let makefile_exists = project_root.join("Makefile").exists();
    let go_mod_exists = project_root.join("go.mod").exists();

    let check = |exists: bool| {
        if exists {
            "✓".green().to_string()
        } else {
            "✗".dimmed().to_string()
        }
    };

    // Discover all markdown files in root (DX-001: not just 4 hardcoded names)
    let mut discovered_md: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") && entry.path().is_file() {
                discovered_md.push(name);
            }
        }
    }
    discovered_md.sort();

    // Display discovered markdown files
    if discovered_md.is_empty() {
        println!("  {} *.md (no markdown files found)", "✗".dimmed());
    } else {
        for md in &discovered_md {
            println!("  {} {}", "✓".green(), md);
        }
    }

    // Display project manifest files
    println!("  {} package.json", check(package_json_exists));
    println!("  {} Cargo.toml", check(cargo_toml_exists));
    if requirements_txt_exists || pyproject_toml_exists || setup_py_exists || pipfile_exists {
        println!("  {} requirements.txt", check(requirements_txt_exists));
        println!("  {} pyproject.toml", check(pyproject_toml_exists));
        println!("  {} setup.py", check(setup_py_exists));
        println!("  {} Pipfile", check(pipfile_exists));
    }
    if makefile_exists {
        println!("  {} Makefile", check(makefile_exists));
    }
    if go_mod_exists {
        println!("  {} go.mod", check(go_mod_exists));
    }
    println!("  {} .git", check(git_exists));

    // Check for docs/ directory
    let docs_dir = project_root.join("docs");
    if docs_dir.is_dir() {
        let doc_count = std::fs::read_dir(&docs_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
                    .count()
            })
            .unwrap_or(0);
        if doc_count > 0 {
            println!("  {} docs/ ({} markdown files)", "✓".green(), doc_count);
        }
    }

    // Detect project type
    let project_type = if cargo_toml_exists {
        "Rust"
    } else if package_json_exists {
        "JavaScript/TypeScript"
    } else if pyproject_toml_exists || requirements_txt_exists || setup_py_exists || pipfile_exists
    {
        "Python"
    } else if go_mod_exists {
        "Go"
    } else {
        "Unknown"
    };
    println!(
        "\n  {} Detected project type: {}",
        "→".cyan(),
        project_type.cyan()
    );
    println!();

    let spec_exists = discovered_md.iter().any(|n| n == "SPEC.md");

    // Step 1: Detect tools
    println!("{}", "Step 2: Tool Detection".bold());
    let tools = detect_tools();
    display_detected_tools(&tools);
    println!();

    // Step 2: Scaffold .forge/ directory
    println!("{}", "Step 3: Scaffolding .forge/".bold());
    scaffold_forge_dir(&forge_dir)?;

    // Step 3: Create initial state
    let state_mgr = StateManager::new(&forge_dir);
    let state = ForgeState {
        project_name: project_name.clone(),
        tools,
        ..Default::default()
    };
    state_mgr.save(&state)?;
    println!("  {} state.json", "✓".green());

    // Step 4: Create plan template if no SPEC.md
    let plan_mgr = PlanManager::new(&forge_dir);
    if spec_exists {
        println!(
            "\n  {} SPEC.md detected — use {} to generate plan from spec.",
            "→".cyan(),
            "forge plan".cyan()
        );
    } else {
        let template = PlanManager::generate_template(&project_name);
        plan_mgr.write_plan(&template)?;
        println!("  {} plan.md (template)", "✓".green());
    }

    // Step 5: Log the event
    let event_logger = EventLogger::new(&forge_dir);
    event_logger.log(&ForgeEvent::new(
        EventType::PlanCreated,
        format!("Forge initialized for project: {project_name}"),
    ))?;
    println!("  {} events.jsonl", "✓".green());

    // Summary
    println!("\n{}", "=".repeat(40));
    println!(
        "{} Forge initialized for {}",
        "✓".green().bold(),
        project_name.cyan().bold()
    );
    println!();
    println!("Next steps:");
    println!(
        "  {} — choose the AI brain (rule-based is free, openai needs API key)",
        "forge config brain openai".cyan()
    );
    println!(
        "  {} — generate tasks from your SPEC.md or project context",
        "forge plan --generate".cyan()
    );
    println!("  {} — see the task board", "forge status".cyan());
    println!();

    Ok(())
}

fn scaffold_forge_dir(forge_dir: &Path) -> anyhow::Result<()> {
    let dirs = [
        "",
        "tasks",
        "knowledge",
        "knowledge/decisions",
        "knowledge/learnings",
        "knowledge/research",
        "knowledge/patterns",
        "results",
    ];

    for dir in &dirs {
        let path = forge_dir.join(dir);
        std::fs::create_dir_all(&path)?;
    }

    // Create .gitignore for worktrees
    std::fs::write(
        forge_dir.join(".gitignore"),
        "# Forge-managed directories\nworktrees/\nresults/\n",
    )?;

    // Scaffold governance.json — the shared state bus between all 3 products (DX-002)
    let governance = serde_json::json!({
        "version": "1.0.0",
        "created_at": chrono::Utc::now().to_rfc3339(),
        "constitution": {
            "principles": [],
            "constraints": []
        },
        "workstreams": [],
        "audit_trail": []
    });
    std::fs::write(
        forge_dir.join("governance.json"),
        serde_json::to_string_pretty(&governance)?,
    )?;

    println!("  {} .forge/ directory scaffolded", "✓".green());
    Ok(())
}
