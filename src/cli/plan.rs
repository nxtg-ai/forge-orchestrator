use crate::brain::ForgeBrain;
use crate::brain::openai::OpenAIBrain;
use crate::brain::rule_based::RuleBasedBrain;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::finding::{FindingManager, FindingSeverity, FindingType};
use crate::core::plan::PlanManager;
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskPhase};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;

fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .template("  {spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&[
            "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
            "\u{2827}", "\u{2807}", "\u{280f}", " ",
        ])
}

fn new_spinner(msg: &str) -> ProgressBar {
    let sp = ProgressBar::new_spinner();
    sp.set_style(spinner_style());
    sp.set_message(msg.to_string());
    sp.enable_steady_tick(std::time::Duration::from_millis(80));
    sp
}

/// Finish spinner and print message to stdout (spinner output is unreliable in non-TTY).
fn finish_spinner(sp: ProgressBar, msg: &str) {
    sp.finish_and_clear();
    println!("  {msg}");
}

pub fn execute(
    project_root: &Path,
    generate: bool,
    spec_path: Option<String>,
    from_findings: bool,
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

    if from_findings {
        return generate_from_findings(project_root);
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

    // ── Phase 1: Load spec ──────────────────────────────────────
    let sp = new_spinner("Loading spec...");
    let (spec_content, source_label) = resolve_plan_input(project_root, spec_path)?;
    finish_spinner(
        sp,
        &format!(
            "{} Spec loaded ({} lines) from {}",
            "✓".green(),
            spec_content.lines().count(),
            source_label
        ),
    );

    // ── Phase 2: Scan codebase ──────────────────────────────────
    let sp = new_spinner("Scanning codebase...");
    let (inventory, file_count) = scan_codebase(project_root);
    if file_count > 0 {
        finish_spinner(
            sp,
            &format!(
                "{} Codebase scanned ({} source files)",
                "✓".green(),
                file_count
            ),
        );
    } else {
        finish_spinner(
            sp,
            &format!(
                "{} No source files found (greenfield project)",
                "—".dimmed()
            ),
        );
    }

    // Combine spec + inventory for the brain
    let brain_input = if inventory.is_empty() {
        spec_content.clone()
    } else {
        format!(
            "{spec_content}\n\n---\n\n\
             EXISTING CODEBASE INVENTORY:\n\n{inventory}\n\n---\n\n\
             Generate tasks ONLY for what is missing, incomplete, or needs updating.\n\
             Do NOT generate tasks for features that already exist in the codebase."
        )
    };

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

    let tools_for_brain = if available_tools.is_empty() {
        vec![AgentType::Any]
    } else {
        available_tools.clone()
    };

    // ── Phase 3: Decompose into tasks ───────────────────────────
    let task_mgr = TaskManager::new(forge_dir);
    let start_id = task_mgr.next_task_number()?;

    if start_id > 1 {
        println!(
            "  {} Existing tasks found (up to T-{:03}). New tasks start at T-{:03}.",
            "\u{2192}".cyan(),
            start_id - 1,
            start_id
        );
    }

    let sp = new_spinner("Decomposing spec into tasks...");
    let mut tasks = brain.decompose_plan(&brain_input, &tools_for_brain, start_id)?;
    finish_spinner(
        sp,
        &format!("{} Generated {} tasks", "✓".green(), tasks.len()),
    );

    // ── Phase 4: Assign agents ──────────────────────────────────
    let sp = new_spinner("Assigning agents to tasks...");
    for task in &mut tasks {
        let assigned = brain.assign_task(task, &tools_for_brain)?;
        task.assigned_to = Some(assigned);
    }
    finish_spinner(sp, &format!("{} Agents assigned", "✓".green()));

    // Stamp plan version on all new tasks
    let plan_version = if start_id > 1 {
        let existing = task_mgr.list_tasks()?;
        existing
            .iter()
            .filter_map(|t| t.plan_version)
            .max()
            .unwrap_or(1)
            + 1
    } else {
        1
    };
    for task in &mut tasks {
        task.plan_version = Some(plan_version);
    }

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

    // ── Phase 5: Write to disk ──────────────────────────────────
    let sp = new_spinner("Writing task board...");
    let task_mgr = TaskManager::new(forge_dir);
    for task in &tasks {
        task_mgr.create_task(task)?;
    }

    let plan_mgr = PlanManager::new(forge_dir);
    let plan_content = generate_plan_markdown(&state.project_name, &tasks);
    plan_mgr.write_plan(&plan_content)?;

    let event_logger = EventLogger::new(forge_dir);
    event_logger.log(&ForgeEvent::new(
        EventType::PlanCreated,
        format!("Plan generated from {source_label}: {} tasks", tasks.len()),
    ))?;

    let summary = crate::core::state::TaskSummary {
        total: tasks.len(),
        pending: tasks.len(),
        ..Default::default()
    };
    state_mgr.update_task_summary(summary)?;
    finish_spinner(
        sp,
        &format!("{} Plan written to .forge/plan.md", "✓".green()),
    );

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

fn generate_from_findings(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    let finding_mgr = FindingManager::new(&forge_dir);
    let task_mgr = TaskManager::new(&forge_dir);

    let findings = finding_mgr.list_findings()?;
    let actionable: Vec<_> = findings
        .iter()
        .filter(|f| !matches!(f.severity, FindingSeverity::Positive))
        .collect();

    if actionable.is_empty() {
        println!(
            "{} No actionable findings. Run {} first.",
            "!".yellow(),
            "forge uat".cyan()
        );
        return Ok(());
    }

    let mut next_num = task_mgr.next_task_number()?;

    // Determine plan version (max existing + 1)
    let plan_version = {
        let existing = task_mgr.list_tasks()?;
        existing
            .iter()
            .filter_map(|t| t.plan_version)
            .max()
            .unwrap_or(0)
            + 1
    };

    let mut generated = Vec::new();

    for finding in &actionable {
        let task_type = match finding.finding_type {
            FindingType::Bug => "implement",
            FindingType::Missing => "implement",
            FindingType::Enhancement => "implement",
            FindingType::Positive => continue,
        };

        let priority = match finding.severity {
            FindingSeverity::Critical => "P0",
            FindingSeverity::High => "P1",
            FindingSeverity::Medium => "P2",
            FindingSeverity::Low => "P3",
            FindingSeverity::Positive => continue,
        };

        let task_id = format!("T-{next_num:03}");
        let mut task = Task::new(
            &task_id,
            format!("Fix: {}", truncate(&finding.description, 60)),
            format!(
                "UAT Finding {}: {}\n\nSeverity: {}\nPriority: {}\nRelated tasks: {}\n\nFix this issue.",
                finding.id,
                finding.description,
                finding.severity,
                priority,
                if finding.related_tasks.is_empty() {
                    "none".to_string()
                } else {
                    finding.related_tasks.join(", ")
                }
            ),
        );
        task.task_type = Some(task_type.to_string());
        task.phase = Some(TaskPhase::Fix);
        task.assigned_to = Some(AgentType::Claude);
        task.plan_version = Some(plan_version);

        task_mgr.create_task(&task)?;
        generated.push((task_id, finding));
        next_num += 1;
    }

    println!();
    println!("{}", "Fix tasks generated from UAT findings:".bold());
    for (task_id, finding) in &generated {
        let sev = match finding.severity {
            FindingSeverity::Critical => "CRIT".red().bold().to_string(),
            FindingSeverity::High => "HIGH".red().to_string(),
            FindingSeverity::Medium => "MED".yellow().to_string(),
            _ => "LOW".dimmed().to_string(),
        };
        println!(
            "  {} {} [{}] {}",
            "\u{25cb}".cyan(),
            task_id,
            sev,
            truncate(&finding.description, 50)
        );
    }
    println!();
    println!(
        "  {} fix tasks created. Run {} or {} to execute.",
        generated.len(),
        "forge dashboard".cyan(),
        "forge run".cyan()
    );
    println!();

    Ok(())
}

fn generate_plan_markdown(project_name: &str, tasks: &[crate::core::task::Task]) -> String {
    let mut content = String::new();
    content.push_str(&format!("# {project_name} — Master Plan\n\n"));
    content.push_str("**Generated by:** Forge Orchestrator v1.1.0\n");
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
        let label = spec_file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        return Ok((content, label));
    }

    // No spec file — gather project context instead
    let context = gather_project_context(project_root)?;
    if context.is_empty() {
        anyhow::bail!(
            "No project context found. Create a README.md or SPEC.md, then run `forge plan --generate`."
        );
    }
    Ok((context, "project context".to_string()))
}

/// Gather project context from README, markdown files, and manifest files.
fn gather_project_context(project_root: &Path) -> anyhow::Result<String> {
    let mut sections: Vec<String> = Vec::new();

    // 1. README.md — primary context
    let readme = project_root.join("README.md");
    if readme.exists() {
        let content = std::fs::read_to_string(&readme)?;
        if !content.trim().is_empty() {
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
            sections.push(format!("# {name} ({label})\n\n```\n{truncated}\n```"));
        }
    }

    // 3. Other markdown files in root
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
                let truncated: String = content.lines().take(100).collect::<Vec<_>>().join("\n");
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
                let truncated: String = content.lines().take(80).collect::<Vec<_>>().join("\n");
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
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

// ─── DX-017: Codebase Scanner ───────────────────────────────────────────────

/// Directories to skip during codebase scanning.
const IGNORE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".forge",
    ".next",
    ".nuxt",
    "vendor",
    "venv",
    ".venv",
    "coverage",
];

/// Source file extensions to include.
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "kt", "swift", "rb", "cs", "cpp", "c", "h",
    "hpp",
];

/// Test file patterns.
const TEST_PATTERNS: &[&str] = &["test", "spec", "_test.", ".test.", ".spec."];

/// Export/definition patterns by extension for signature extraction.
fn export_patterns(ext: &str) -> &'static [&'static str] {
    match ext {
        "ts" | "tsx" | "js" | "jsx" => &[
            "export class ",
            "export function ",
            "export const ",
            "export interface ",
            "export type ",
            "export default ",
            "export enum ",
        ],
        "rs" => &[
            "pub fn ",
            "pub struct ",
            "pub enum ",
            "pub trait ",
            "pub mod ",
            "pub type ",
        ],
        "py" => &["class ", "def ", "__all__"],
        "go" => &["func ", "type "],
        _ => &[],
    }
}

/// Scan the project codebase and build a concise inventory.
/// Returns (inventory_text, source_file_count).
fn scan_codebase(project_root: &Path) -> (String, usize) {
    let mut source_files: Vec<(String, usize, Vec<String>)> = Vec::new(); // (path, lines, exports)
    let mut test_files: Vec<(String, usize)> = Vec::new(); // (path, lines)
    let mut total_chars = 0usize;
    const MAX_CHARS: usize = 12_000; // ~4000 tokens

    let walker = walkdir::WalkDir::new(project_root)
        .max_depth(4) // 3 levels below root
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Skip ignored directories
            if e.file_type().is_dir() {
                return !IGNORE_DIRS.contains(&name.as_ref());
            }
            true
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) if SOURCE_EXTENSIONS.contains(&e) => e,
            _ => continue,
        };

        // Relative path from project root
        let rel_path = path
            .strip_prefix(project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Count lines
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let line_count = content.lines().count();

        // Check if test file
        let rel_lower = rel_path.to_lowercase();
        if TEST_PATTERNS.iter().any(|p| rel_lower.contains(p)) {
            let entry_str = format!("{rel_path} ({line_count} lines)\n");
            total_chars += entry_str.len();
            test_files.push((rel_path, line_count));
            if total_chars > MAX_CHARS {
                break;
            }
            continue;
        }

        // Extract export signatures
        let patterns = export_patterns(ext);
        let exports: Vec<String> = if patterns.is_empty() {
            Vec::new()
        } else {
            content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    patterns.iter().any(|p| trimmed.starts_with(p))
                })
                .take(10) // max 10 exports per file
                .map(|line| {
                    let trimmed = line.trim();
                    // Truncate long lines
                    if trimmed.len() > 80 {
                        format!("{}...", &trimmed[..77])
                    } else {
                        trimmed.to_string()
                    }
                })
                .collect()
        };

        let entry_str = format!("{rel_path} ({line_count} lines)\n");
        total_chars += entry_str.len();
        total_chars += exports.iter().map(|e| e.len() + 4).sum::<usize>(); // "  - " prefix

        source_files.push((rel_path, line_count, exports));

        if total_chars > MAX_CHARS {
            break;
        }
    }

    let file_count = source_files.len() + test_files.len();
    if file_count == 0 {
        return (String::new(), 0);
    }

    // Build inventory string
    let mut inventory = String::new();

    // Source files
    if !source_files.is_empty() {
        inventory.push_str("## Source Files\n\n");
        for (path, lines, exports) in &source_files {
            inventory.push_str(&format!("- {path} ({lines} lines)"));
            if !exports.is_empty() {
                inventory.push('\n');
                for export in exports {
                    inventory.push_str(&format!("  - {export}\n"));
                }
            } else {
                inventory.push('\n');
            }
        }
    }

    // Test files
    if !test_files.is_empty() {
        inventory.push_str("\n## Test Files\n\n");
        for (path, lines) in &test_files {
            inventory.push_str(&format!("- {path} ({lines} lines)\n"));
        }
    }

    (inventory, file_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::finding::{Finding, FindingSeverity, FindingType};

    fn setup_forge_project(tmp: &std::path::Path) {
        let forge_dir = tmp.join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();
        std::fs::create_dir_all(forge_dir.join("findings")).unwrap();

        // Minimal state.json
        let state = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "tools": [],
            "brain": { "provider": "rule-based" },
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0 },
            "governance": { "file_locks": {} }
        });
        std::fs::write(
            forge_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();
    }

    fn make_finding(
        id: &str,
        desc: &str,
        severity: FindingSeverity,
        finding_type: FindingType,
    ) -> Finding {
        Finding {
            id: id.to_string(),
            description: desc.to_string(),
            severity,
            finding_type,
            related_tasks: vec![],
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_from_findings_generates_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        let forge_dir = tmp.path().join(".forge");
        let finding_mgr = FindingManager::new(&forge_dir);

        finding_mgr
            .save_finding(&make_finding(
                "F-001",
                "Login button broken",
                FindingSeverity::High,
                FindingType::Bug,
            ))
            .unwrap();
        finding_mgr
            .save_finding(&make_finding(
                "F-002",
                "Missing error messages",
                FindingSeverity::Medium,
                FindingType::Missing,
            ))
            .unwrap();

        generate_from_findings(tmp.path()).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].title.contains("Fix:"));
        assert_eq!(tasks[0].phase, Some(TaskPhase::Fix));
        assert_eq!(tasks[0].assigned_to, Some(AgentType::Claude));
    }

    #[test]
    fn test_from_findings_skips_positive() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        let forge_dir = tmp.path().join(".forge");
        let finding_mgr = FindingManager::new(&forge_dir);

        finding_mgr
            .save_finding(&make_finding(
                "F-001",
                "Love the new UI",
                FindingSeverity::Positive,
                FindingType::Positive,
            ))
            .unwrap();
        finding_mgr
            .save_finding(&make_finding(
                "F-002",
                "Button broken",
                FindingSeverity::High,
                FindingType::Bug,
            ))
            .unwrap();

        generate_from_findings(tmp.path()).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].description.contains("F-002"));
    }

    #[test]
    fn test_from_findings_monotonic_ids() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        let forge_dir = tmp.path().join(".forge");

        // Pre-create an existing task
        let task_mgr = TaskManager::new(&forge_dir);
        let existing = Task::new("T-001", "Existing task", "Already here");
        task_mgr.create_task(&existing).unwrap();

        let finding_mgr = FindingManager::new(&forge_dir);
        finding_mgr
            .save_finding(&make_finding(
                "F-001",
                "Bug found",
                FindingSeverity::High,
                FindingType::Bug,
            ))
            .unwrap();

        generate_from_findings(tmp.path()).unwrap();

        let tasks = task_mgr.list_tasks().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].id, "T-002"); // Appended after existing
    }

    #[test]
    fn test_from_findings_severity_mapping() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        let forge_dir = tmp.path().join(".forge");
        let finding_mgr = FindingManager::new(&forge_dir);

        finding_mgr
            .save_finding(&make_finding(
                "F-001",
                "Security crash",
                FindingSeverity::Critical,
                FindingType::Bug,
            ))
            .unwrap();
        finding_mgr
            .save_finding(&make_finding(
                "F-002",
                "Minor typo",
                FindingSeverity::Low,
                FindingType::Bug,
            ))
            .unwrap();

        generate_from_findings(tmp.path()).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();

        // Critical → P0 in description
        assert!(tasks[0].description.contains("P0"));
        // Low → P3 in description
        assert!(tasks[1].description.contains("P3"));
    }

    #[test]
    fn test_from_findings_empty_findings() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        // No findings → should not create tasks, just print message
        generate_from_findings(tmp.path()).unwrap();

        let forge_dir = tmp.path().join(".forge");
        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_from_findings_has_fix_phase() {
        let tmp = tempfile::tempdir().unwrap();
        setup_forge_project(tmp.path());

        let forge_dir = tmp.path().join(".forge");
        let finding_mgr = FindingManager::new(&forge_dir);

        finding_mgr
            .save_finding(&make_finding(
                "F-001",
                "Improvement needed",
                FindingSeverity::Medium,
                FindingType::Enhancement,
            ))
            .unwrap();

        generate_from_findings(tmp.path()).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].phase, Some(TaskPhase::Fix));
        assert_eq!(tasks[0].task_type.as_deref(), Some("implement"));
    }
}
