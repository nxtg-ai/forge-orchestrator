use crate::cli::doctor::{Preflight, release_preflight};
use crate::core::ship;
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, auto: bool, dry_run: bool) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        anyhow::bail!("Forge is not initialized. Run `forge init` first.");
    }

    println!("{}", "FORGE SHIP — Post-UAT Wrap-Up".bold());
    println!("{}", "=".repeat(40));
    println!();

    // Step 0: release-debt preflight. Ship is the moment version surfaces must agree — shipping
    // with a desynced lockfile is how the v1.5.1 train produced a --locked build failure.
    // FAIL blocks; WARN is surfaced and allowed, since unreleased-commit debt is often exactly
    // what this command is about to discharge.
    println!("{}", "Step 0: Release Preflight".bold());
    match release_preflight(project_root) {
        Preflight::Blocked(details) => {
            for detail in &details {
                println!("  {} {detail}", "✗".red());
            }
            anyhow::bail!(
                "release preflight FAILED — fix the version surfaces above, or run `forge doctor` for the full report"
            );
        }
        Preflight::Warned(details) => {
            for detail in &details {
                println!("  {} {detail}", "!".yellow());
            }
        }
        Preflight::Clean(summary) => {
            println!("  {} {summary}", "✓".green());
        }
        Preflight::Skipped(reason) => {
            println!("  {} {reason}", "-".dimmed());
        }
    }
    println!();

    // Step 1: Generate changelog
    println!("{}", "Step 1: Generate Changelog".bold());
    let changelog = ship::generate_changelog(&forge_dir)?;
    println!("{changelog}");

    if !dry_run {
        // Append to project CHANGELOG.md if it exists
        let changelog_path = project_root.join("CHANGELOG.md");
        if changelog_path.exists() {
            let existing = std::fs::read_to_string(&changelog_path)?;
            // Insert after the first heading line
            if let Some(pos) = existing.find("\n## ") {
                let mut new_content = existing[..pos].to_string();
                new_content.push('\n');
                new_content.push_str(&changelog);
                new_content.push_str(&existing[pos..]);
                std::fs::write(&changelog_path, new_content)?;
                println!("  {} Appended to CHANGELOG.md", "✓".green());
            } else {
                let mut new_content = existing;
                new_content.push('\n');
                new_content.push_str(&changelog);
                std::fs::write(&changelog_path, new_content)?;
                println!("  {} Appended to CHANGELOG.md", "✓".green());
            }
        } else {
            println!(
                "  {} No CHANGELOG.md found — changelog printed above",
                "→".cyan()
            );
        }
    } else {
        println!("  {} Dry run — skipping write", "→".cyan());
    }
    println!();

    // Step 2: Version bump suggestion
    println!("{}", "Step 2: Version Bump Suggestion".bold());
    let bump = ship::suggest_version_bump(&forge_dir);
    println!("  Suggested bump: {}", format!("{bump}").cyan());
    println!();

    // Step 3: Archive cycle
    println!("{}", "Step 3: Archive Build Artifacts".bold());
    if !dry_run {
        let (count, archive_path) = ship::archive_cycle(&forge_dir)?;
        println!(
            "  {} Archived {} artifacts to {}",
            "✓".green(),
            count,
            archive_path.display()
        );
    } else {
        println!("  {} Dry run — skipping archive", "→".cyan());
    }
    println!();

    // Step 4: Clean state
    println!("{}", "Step 4: Clean State".bold());
    if !dry_run {
        if auto {
            ship::clean_state(&forge_dir)?;
            println!("  {} State cleaned — ready for next cycle", "✓".green());
        } else {
            println!("  Run with --auto to clean state automatically.");
            println!("  Or run {} manually.", "forge ship --auto".cyan());
        }
    } else {
        println!("  {} Dry run — skipping clean", "→".cyan());
    }

    println!();
    println!("{}", "=".repeat(40));
    if dry_run {
        println!(
            "{} Dry run complete. Run {} to execute.",
            "→".cyan(),
            "forge ship".cyan()
        );
    } else {
        println!(
            "{} Ship phase complete. Ready for next cycle.",
            "✓".green().bold()
        );
    }

    Ok(())
}
