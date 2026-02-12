use crate::core::finding::{classify_finding, find_related_tasks, Finding, FindingManager};
use crate::core::task::TaskManager;
use colored::Colorize;
use std::io::{self, BufRead, Write};
use std::path::Path;

pub fn execute(project_root: &Path) -> anyhow::Result<()> {
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
    let finding_mgr = FindingManager::new(&forge_dir);
    let tasks = task_mgr.list_tasks()?;

    println!();
    println!(
        "{}",
        "UAT Mode \u{2014} Describe issues naturally. Type 'done' when finished.".bold()
    );
    println!();

    // Show acceptance criteria from completed tasks
    let criteria: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == crate::core::task::TaskStatus::Completed)
        .flat_map(|t| {
            t.acceptance_criteria
                .iter()
                .map(move |c| (t.id.clone(), c.clone()))
        })
        .collect();

    if !criteria.is_empty() {
        println!(
            "  {}:",
            "Acceptance Criteria (from completed tasks)".dimmed()
        );
        for (task_id, criterion) in &criteria {
            println!("  {} {} ({})", "[ ]".dimmed(), criterion, task_id.dimmed());
        }
        println!();
    }

    let mut findings: Vec<Finding> = Vec::new();
    let mut next_num = finding_mgr.next_finding_number()?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("  {} ", ">".cyan());
        stdout.flush()?;

        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }
        if input.eq_ignore_ascii_case("done")
            || input.eq_ignore_ascii_case("quit")
            || input.eq_ignore_ascii_case("exit")
        {
            break;
        }

        let (severity, finding_type) = classify_finding(input);
        let related = find_related_tasks(input, &tasks);

        let finding = Finding {
            id: format!("F-{next_num:03}"),
            description: input.to_string(),
            severity: severity.clone(),
            finding_type: finding_type.clone(),
            related_tasks: related.clone(),
            created_at: chrono::Utc::now(),
        };

        finding_mgr.save_finding(&finding)?;
        findings.push(finding);
        next_num += 1;

        let related_str = if related.is_empty() {
            String::new()
        } else {
            format!(", related: {}", related.join(", "))
        };

        let severity_colored = match &severity {
            crate::core::finding::FindingSeverity::Critical => {
                "critical".red().bold().to_string()
            }
            crate::core::finding::FindingSeverity::High => "high".red().to_string(),
            crate::core::finding::FindingSeverity::Medium => "medium".yellow().to_string(),
            crate::core::finding::FindingSeverity::Low => "low".dimmed().to_string(),
            crate::core::finding::FindingSeverity::Positive => "positive".green().to_string(),
        };

        println!(
            "  {} (severity: {}{})",
            "Captured".green(),
            severity_colored,
            related_str,
        );
    }

    // Summary
    println!();
    let bugs = findings
        .iter()
        .filter(|f| {
            !matches!(
                f.severity,
                crate::core::finding::FindingSeverity::Positive
            )
        })
        .count();
    let positives = findings
        .iter()
        .filter(|f| {
            matches!(
                f.severity,
                crate::core::finding::FindingSeverity::Positive
            )
        })
        .count();
    println!(
        "  {} findings captured ({} issues, {} positive).",
        findings.len(),
        bugs,
        positives
    );

    if bugs > 0 {
        println!(
            "  Run {} to generate fix tasks.",
            "forge plan --from-findings".cyan()
        );
    }
    println!();

    Ok(())
}
