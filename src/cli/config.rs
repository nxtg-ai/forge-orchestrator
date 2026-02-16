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
        "claude.auth" | "codex.auth" | "gemini.auth" => {
            let agent = key.split('.').next().unwrap();
            match value {
                "subscription" | "api" => {
                    state
                        .agent_auth
                        .insert(agent.to_string(), value.to_string());
                    state.updated_at = chrono::Utc::now();
                    state_mgr.save(&state)?;
                    println!("✓ {agent} auth mode set to: {value}");
                    match value {
                        "subscription" => {
                            println!(
                                "  \u{2192} Will use CLI subscription (API keys stripped from subprocess)"
                            );

                            // DX-038: Risk warning based on RESEARCH-001
                            if agent == "claude" {
                                println!();
                                println!(
                                    "  \u{26A0}\u{26A0}\u{26A0}  WARNING: SUBSCRIPTION RISK  \u{26A0}\u{26A0}\u{26A0}"
                                );
                                println!(
                                    "  Anthropic ACTIVELY BLOCKS subscription-based CLI orchestration."
                                );
                                println!(
                                    "  In January 2026, accounts were banned for using third-party"
                                );
                                println!(
                                    "  tools with subscription OAuth tokens (ToS Section D.4)."
                                );
                                println!(
                                    "  \u{2192} STRONGLY RECOMMENDED: Use `forge config claude.auth api` instead."
                                );
                                println!(
                                    "  \u{2192} See: docs/research/cli-subscription-gating-analysis.md"
                                );
                                println!();
                                println!(
                                    "  To use subscription mode in dashboard/start, you must pass:"
                                );
                                println!("  `forge dashboard --i-accept-subscription-risk`");
                            } else if agent == "codex" {
                                println!();
                                println!(
                                    "  \u{26A0} CAUTION: Codex subscription has 10-60 cloud task/5h limits."
                                );
                                println!("  Consider API key mode for heavier workloads.");
                            }
                        }
                        "api" => {
                            println!("  \u{2192} Will pass API keys to subprocess");
                        }
                        _ => {}
                    }
                }
                _ => {
                    anyhow::bail!(
                        "Unknown auth mode: {value}. Choose: subscription, api\n\n  subscription — Use CLI subscription (Pro/Max). Strips API keys from subprocess\n  api          — Pass API keys through to subprocess"
                    );
                }
            }
        }
        "claude.permissions" | "codex.permissions" | "gemini.permissions" => {
            let agent = key.split('.').next().unwrap();
            match value {
                "safe" | "yolo" => {
                    state
                        .agent_permissions
                        .insert(agent.to_string(), value.to_string());
                    state.updated_at = chrono::Utc::now();
                    state_mgr.save(&state)?;
                    println!("✓ {agent} permissions set to: {value}");
                    match value {
                        "safe" => {
                            println!(
                                "  → Read-only mode (agent cannot write files or run commands)"
                            );
                        }
                        "yolo" => {
                            println!(
                                "  → Full autonomy (agent can read, write, edit, and execute)"
                            );
                            println!("  ⚡ YOLO MODE ACTIVATED for {agent}");
                        }
                        _ => {}
                    }
                }
                _ => {
                    anyhow::bail!(
                        "Unknown permission mode: {value}. Choose: safe, yolo\n\n  safe — Read-only (agent can analyze but not modify)\n  yolo — Full autonomy (agent can read, write, edit, execute)"
                    );
                }
            }
        }
        "scheduler.rotation" => match value.to_lowercase().as_str() {
            "true" | "on" | "yes" | "enabled" => {
                state.scheduler.rotation = true;
                state.updated_at = chrono::Utc::now();
                state_mgr.save(&state)?;
                println!("✓ scheduler.rotation = enabled");
                println!("  → Rate-limited tasks will be reassigned to available providers");
            }
            "false" | "off" | "no" | "disabled" => {
                state.scheduler.rotation = false;
                state.updated_at = chrono::Utc::now();
                state_mgr.save(&state)?;
                println!("✓ scheduler.rotation = disabled");
                println!("  → Tasks stay with their assigned provider (default)");
            }
            _ => {
                anyhow::bail!("Invalid value: {value}. Use: enabled / disabled");
            }
        },
        "scheduler.pacing" => {
            // Format: "min-max" e.g. "64-179"
            let parts: Vec<&str> = value.split('-').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid format: {value}. Use: MIN-MAX (e.g. 64-179)");
            }
            let min: u64 = parts[0]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid min: {}", parts[0]))?;
            let max: u64 = parts[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid max: {}", parts[1]))?;
            if min >= max || min < 10 {
                anyhow::bail!("Min must be >= 10 and less than max. Got: {min}-{max}");
            }
            state.scheduler.pacing_min_secs = min;
            state.scheduler.pacing_max_secs = max;
            state.updated_at = chrono::Utc::now();
            state_mgr.save(&state)?;
            println!("✓ Subscription pacing set to: {min}-{max}s");
            println!("  → Random delay of {min}-{max}s between tasks in subscription mode");
        }
        "git.auto_commit" => match value.to_lowercase().as_str() {
            "true" | "on" | "yes" => {
                state.git.auto_commit = true;
                state.updated_at = chrono::Utc::now();
                state_mgr.save(&state)?;
                println!("✓ git.auto_commit = true");
                println!("  → Agents will commit after each task completion");
            }
            "false" | "off" | "no" => {
                state.git.auto_commit = false;
                state.updated_at = chrono::Utc::now();
                state_mgr.save(&state)?;
                println!("✓ git.auto_commit = false");
                println!("  → No automatic commits — you manage git manually");
            }
            _ => {
                anyhow::bail!("Invalid value: {value}. Use: true / false");
            }
        },
        _ => {
            anyhow::bail!(
                "Unknown config key: {key}\n\nAvailable keys:\n  brain                — Brain provider (rule-based, openai)\n  brain.model          — Model name (gpt-4o, gpt-4.1, gpt-5, gpt-5-mini)\n  claude.auth          — Claude auth mode (subscription, api)\n  codex.auth           — Codex auth mode (subscription, api)\n  gemini.auth          — Gemini auth mode (subscription, api)\n  claude.permissions   — Claude permission mode (safe, yolo)\n  codex.permissions    — Codex permission mode (safe, yolo)\n  gemini.permissions   — Gemini permission mode (safe, yolo)\n  scheduler.rotation   — Rotate tasks on rate limit (enabled, disabled)\n  scheduler.pacing     — Subscription delay range (e.g. 64-179)\n  git.auto_commit      — Auto-commit after task completion (true, false)"
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
        println!();
    }

    // Agent config
    println!("  Agent Configuration:");
    for agent in ["claude", "codex", "gemini"] {
        let auth = state
            .agent_auth
            .get(agent)
            .cloned()
            .unwrap_or_else(|| "subscription".to_string());
        let perms = state
            .agent_permissions
            .get(agent)
            .cloned()
            .unwrap_or_else(|| "safe".to_string());
        let perms_icon = if perms == "yolo" { "⚡" } else { "🔒" };
        println!("    {agent:8} auth={auth:13} permissions={perms} {perms_icon}",);
    }
    println!();

    // Scheduler config
    println!("  Scheduler:");
    let rotation_icon = if state.scheduler.rotation {
        "✓"
    } else {
        "✗"
    };
    println!(
        "    rotation     = {} {rotation_icon}",
        state.scheduler.rotation
    );
    println!(
        "    pacing       = {}-{}s (subscription mode)",
        state.scheduler.pacing_min_secs, state.scheduler.pacing_max_secs
    );
    println!();

    // Git config
    println!("  Git:");
    let auto_icon = if state.git.auto_commit { "✓" } else { "✗" };
    println!("    auto_commit  = {} {auto_icon}", state.git.auto_commit);
    println!("    strategy     = {}", state.git.strategy);
    println!();

    // Usage hints (DX-006)
    println!("To change settings:");
    println!("  forge config brain openai          # Switch to OpenAI brain");
    println!("  forge config brain rule-based       # Switch to rule-based brain");
    println!("  forge config brain.model gpt-4.1    # Set model (gpt-4.1, gpt-5, gpt-5-mini)");
    println!("  forge config claude.auth api        # Use API key for Claude");
    println!("  forge config claude.auth subscription  # Use Pro/Max subscription");
    println!("  forge config claude.permissions yolo   # Full autonomy for Claude");
    println!("  forge config claude.permissions safe   # Read-only (default)");
    println!("  forge config git.auto_commit true      # Commit after each task");
    println!("  forge config git.auto_commit false     # Disable auto-commit");

    Ok(())
}
