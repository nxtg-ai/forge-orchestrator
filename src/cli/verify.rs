use crate::core::task::TaskManager;
use colored::Colorize;
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
    let generated = task_mgr.generate_verify_subtasks()?;

    if generated.is_empty() {
        println!("{} No new verify tasks to generate.", "✓".green());
    } else {
        println!();
        println!("{}", "Verify subtasks generated:".bold());
        for task in &generated {
            let parent = task.parent_task.as_deref().unwrap_or("?");
            let agent = task
                .assigned_to
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "auto".into());
            println!(
                "  {} {} [{}] ← parent {}",
                "○".cyan(),
                task.id,
                agent,
                parent
            );
        }
        println!();
        println!(
            "  {} verify tasks created. Run {} or {} to execute.",
            generated.len(),
            "forge dashboard".cyan(),
            "forge run".cyan()
        );
    }
    Ok(())
}
