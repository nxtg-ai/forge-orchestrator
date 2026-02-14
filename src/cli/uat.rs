use crate::core::finding::{classify_finding, find_related_tasks, Finding, FindingManager};
use crate::core::task::TaskManager;
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, inline_finding: Option<String>) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    if let Some(description) = inline_finding {
        return execute_inline(&forge_dir, &description);
    }

    // Launch the TUI
    crate::tui::uat_app::run(forge_dir, project_root.to_path_buf())
}

pub(crate) fn execute_inline(forge_dir: &Path, description: &str) -> anyhow::Result<()> {
    let task_mgr = TaskManager::new(forge_dir);
    let finding_mgr = FindingManager::new(forge_dir);
    let tasks = task_mgr.list_tasks()?;

    let (severity, finding_type) = classify_finding(description);
    let related = find_related_tasks(description, &tasks);
    let next_num = finding_mgr.next_finding_number()?;

    let finding = Finding {
        id: format!("F-{next_num:03}"),
        description: description.to_string(),
        severity: severity.clone(),
        finding_type,
        related_tasks: related.clone(),
        created_at: chrono::Utc::now(),
    };

    finding_mgr.save_finding(&finding)?;

    let severity_colored = match &severity {
        crate::core::finding::FindingSeverity::Critical => "critical".red().bold().to_string(),
        crate::core::finding::FindingSeverity::High => "high".red().to_string(),
        crate::core::finding::FindingSeverity::Medium => "medium".yellow().to_string(),
        crate::core::finding::FindingSeverity::Low => "low".dimmed().to_string(),
        crate::core::finding::FindingSeverity::Positive => "positive".green().to_string(),
    };

    let related_str = if related.is_empty() {
        String::new()
    } else {
        format!(", related: {}", related.join(", "))
    };

    println!(
        "  {} {} (severity: {}{})",
        "Captured".green(),
        finding.id.cyan(),
        severity_colored,
        related_str,
    );

    Ok(())
}
