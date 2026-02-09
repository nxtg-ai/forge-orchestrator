use std::path::Path;

use crate::core::state::StateManager;

/// Execute `forge config` subcommands
pub fn execute(project_root: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        anyhow::bail!("Forge is not initialized. Run `forge init` first.");
    }

    let state_mgr = StateManager::new(&forge_dir);
    let mut state = state_mgr.load()?;

    match key {
        "brain" => {
            match value {
                "rule-based" | "openai" => {
                    state.brain.provider = value.to_string();
                    state.updated_at = chrono::Utc::now();
                    state_mgr.save(&state)?;
                    println!("✓ Brain provider set to: {value}");

                    if value == "openai" {
                        // Check for API key
                        if std::env::var("OPENAI_API_KEY").is_err() {
                            println!("  ⚠ OPENAI_API_KEY not found in environment.");
                            println!("  → Add it to .env or export it in your shell.");
                        } else {
                            println!("  ✓ OPENAI_API_KEY found in environment.");
                        }
                    }
                }
                _ => {
                    anyhow::bail!("Unknown brain provider: {value}. Choose: rule-based, openai");
                }
            }
        }
        "brain.model" => {
            state.brain.model = Some(value.to_string());
            state.updated_at = chrono::Utc::now();
            state_mgr.save(&state)?;
            println!("✓ Brain model set to: {value}");
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: {key}\n\nAvailable keys:\n  brain         — Brain provider (rule-based, openai)\n  brain.model   — Model name (gpt-4o, gpt-4.1, gpt-5, gpt-5-mini, gpt-5.3-codex)"
            );
        }
    }

    Ok(())
}

/// Show current config
pub fn show(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        anyhow::bail!("Forge is not initialized. Run `forge init` first.");
    }

    let state_mgr = StateManager::new(&forge_dir);
    let state = state_mgr.load()?;

    println!("FORGE CONFIGURATION");
    println!("========================================");
    println!();
    println!("  brain          = {}", state.brain.provider);
    println!(
        "  brain.model    = {}",
        state.brain.model.as_deref().unwrap_or("(default)")
    );
    println!("  project        = {}", state.project_name);
    println!(
        "  tools          = {}",
        state
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();

    // Check API key status
    if state.brain.provider == "openai" {
        if std::env::var("OPENAI_API_KEY").is_ok() {
            println!("  ✓ OPENAI_API_KEY: set");
        } else {
            println!("  ⚠ OPENAI_API_KEY: NOT set — brain will fall back to rule-based");
        }
    }

    Ok(())
}
