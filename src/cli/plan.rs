use crate::core::plan::PlanManager;
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

    let plan_mgr = PlanManager::new(&forge_dir);

    if plan_mgr.has_plan() {
        let content = plan_mgr.read_plan()?;
        println!("{content}");
    } else {
        println!(
            "{} No plan found. Create one at {}",
            "!".yellow(),
            ".forge/plan.md".cyan()
        );
        println!();
        println!("Tip: Place a SPEC.md in the project root, then run:");
        println!("  {} to auto-generate a plan from it.", "forge plan".cyan());
    }

    Ok(())
}
