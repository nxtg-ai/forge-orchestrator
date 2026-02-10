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

    // Scan for existing files
    let spec_exists = project_root.join("SPEC.md").exists();
    let claude_md_exists = project_root.join("CLAUDE.md").exists();
    let agents_md_exists = project_root.join("AGENTS.md").exists();
    let gemini_md_exists = project_root.join("GEMINI.md").exists();
    let readme_exists = project_root.join("README.md").exists();
    let package_json_exists = project_root.join("package.json").exists();
    let cargo_toml_exists = project_root.join("Cargo.toml").exists();
    let git_exists = project_root.join(".git").exists();
    // Python project markers
    let requirements_txt_exists = project_root.join("requirements.txt").exists();
    let pyproject_toml_exists = project_root.join("pyproject.toml").exists();
    let setup_py_exists = project_root.join("setup.py").exists();
    let pipfile_exists = project_root.join("Pipfile").exists();
    let makefile_exists = project_root.join("Makefile").exists();
    // Go project markers
    let go_mod_exists = project_root.join("go.mod").exists();

    let check = |exists: bool| {
        if exists {
            "✓".green().to_string()
        } else {
            "✗".dimmed().to_string()
        }
    };

    println!("  {} SPEC.md", check(spec_exists));
    println!("  {} CLAUDE.md", check(claude_md_exists));
    println!("  {} AGENTS.md", check(agents_md_exists));
    println!("  {} GEMINI.md", check(gemini_md_exists));
    println!("  {} README.md", check(readme_exists));
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
    println!("  {} — see the task board", "forge status".cyan());
    println!("  {} — create/view the master plan", "forge plan".cyan());
    println!(
        "  {} — run a task headlessly",
        "forge run --task T-001 --agent claude".cyan()
    );
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

    println!("  {} .forge/ directory scaffolded", "✓".green());
    Ok(())
}
