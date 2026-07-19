#![allow(dead_code)] // Phase 1: many trait methods designed for future phases

mod adapters;
mod brain;
mod cli;
mod core;
mod detect;
mod fleet;
mod mcp;
mod pod;
pub(crate) mod tui;

use clap::Parser;
use cli::{Cli, Commands};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env from CWD first (default dotenvy behavior)
    dotenvy::dotenv().ok();

    // Initialize tracing — only active when RUST_LOG is set.
    // Writes to stderr by default; also writes to .forge/debug.log when active.
    init_tracing();

    let cli = Cli::parse();

    // Project-binding priority (DIRECTIVE-16 — per-invocation authority):
    //   1. an explicit `--project` (anything but the `.` default) — authoritative,
    //   2. else `FORGE_PROJECT_ROOT` — the per-connection binding the plugin's `.mcp.json` sets
    //      (previously read NOWHERE, so the contract was silently dropped) — authoritative,
    //   3. else the current directory (`.`) — unbound; a server started this way may still switch
    //      via `forge_set_project` (global active-project as default-when-unspecified only).
    // An authoritative binding must never be overridden by another consumer's `set_project`.
    let (raw_project, explicit_binding) = if cli.project != "." {
        (cli.project.clone(), true)
    } else if let Some(env_root) = std::env::var_os("FORGE_PROJECT_ROOT").filter(|v| !v.is_empty())
    {
        (env_root.to_string_lossy().into_owned(), true)
    } else {
        (cli.project.clone(), false)
    };
    let project_root = PathBuf::from(&raw_project)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&raw_project));

    // Also try loading .env from the project root (for --project /other/dir usage)
    dotenvy::from_path(project_root.join(".env")).ok();
    // And from forge's own config dir (~/.forge/.env)
    if let Ok(home) = std::env::var("HOME") {
        dotenvy::from_path(PathBuf::from(&home).join(".forge").join(".env")).ok();
    }

    match cli.command {
        Commands::Init { name } => {
            cli::init::execute(&project_root, name)?;
        }
        Commands::Plan {
            generate,
            spec,
            from_findings,
        } => {
            cli::plan::execute(&project_root, generate, spec, from_findings)?;
        }
        Commands::Status {
            events,
            budget,
            json,
            all,
        } => {
            if budget {
                // Fleet-wide and project-independent — deliberately NOT gated on `.forge/`.
                let rows = fleet::read_fleet();
                if json {
                    println!("{}", fleet::render_json(&rows));
                } else {
                    print!("{}", fleet::render_table(&rows, all));
                }
            } else {
                cli::status::execute(&project_root, events)?;
            }
        }
        Commands::Run {
            task,
            agent,
            parallel,
            dry_run,
        } => {
            if let (Some(task_id), Some(agent_name)) = (&task, &agent) {
                cli::run::execute(&project_root, task_id, agent_name).await?;
            } else if task.is_none() && agent.is_none() {
                cli::run::execute_all(&project_root, parallel, dry_run).await?;
            } else {
                anyhow::bail!(
                    "Specify both --task and --agent for single-task mode, or omit both for autonomous mode."
                );
            }
        }
        Commands::Start {
            agent,
            r#loop,
            accept_subscription_risk,
        } => {
            // DX-038: Check subscription risk before starting
            if !accept_subscription_risk {
                let forge_dir = project_root.join(".forge");
                if forge_dir.exists() {
                    let state_mgr = cli::start::load_state_for_risk_check(&forge_dir);
                    if let Some(warning) = state_mgr {
                        println!("{warning}");
                        return Ok(());
                    }
                }
            }
            if r#loop {
                cli::start::execute_loop(&project_root, agent.as_deref()).await?;
            } else {
                cli::start::execute(&project_root, agent.as_deref()).await?;
            }
        }
        Commands::Pod { verb } => {
            use cli::PodVerb;
            // Pod verbs carry cosmux-parity exit codes (2 = uncovered preflight targets), so the
            // verdict is the process status, as it is for `forge doctor`.
            let result = match verb {
                PodVerb::Start {
                    name,
                    force,
                    attach,
                } => cli::pod::start(&name, force, attach),
                PodVerb::Stop { name } => cli::pod::stop(&name),
                PodVerb::List => cli::pod::list(),
                PodVerb::Validate { name } => cli::pod::validate(&name),
                PodVerb::Show { name } => cli::pod::show(&name),
                PodVerb::State => cli::pod::show_state(),
                PodVerb::Ps => cli::pod::ps(),
                PodVerb::Gc => cli::pod::gc(),
                PodVerb::Reload { name, force } => cli::pod::reload(&name, force),
                PodVerb::Completions { shell } => cli::pod::completions(shell),
                PodVerb::Preflight { pod, against } => cli::pod::preflight_cmd(
                    pod.as_deref(),
                    against.as_deref().map(std::path::Path::new),
                ),
                PodVerb::Adopt { repair, abort } => {
                    if repair {
                        crate::pod::adopt::repair()
                    } else if abort {
                        crate::pod::adopt::abort()
                    } else {
                        crate::pod::adopt::adopt()
                    }
                }
                PodVerb::Unadopt => crate::pod::adopt::unadopt(),
                PodVerb::PaneRecover { session } => cli::pod::pane_recover(&session),
                PodVerb::AfterDetach { session } => cli::pod::after_detach(&session),
            };
            use std::io::Write;
            let code = match result {
                Ok(code) => code,
                Err(error) => {
                    // Operational errors exit 1, matching cosmux; a check that RAN and found
                    // problems exits 2 via its own return value.
                    eprintln!("forge pod: {error}");
                    1
                }
            };
            std::io::stdout().flush().ok();
            std::process::exit(code);
        }
        Commands::Doctor { strict, json } => {
            let code = cli::doctor::execute(&project_root, strict, json)?;
            // Fail-closed: the verdict IS the exit status, so CI can gate on it directly.
            // Flush explicitly — process::exit does not run destructors or flush stdout.
            use std::io::Write;
            std::io::stdout().flush().ok();
            std::process::exit(code);
        }
        Commands::Verify => {
            cli::verify::execute(&project_root)?;
        }
        Commands::Uat { finding } => {
            cli::uat::execute(&project_root, finding)?;
        }
        Commands::Sync => {
            cli::sync::execute(&project_root)?;
        }
        Commands::Mcp => {
            cli::mcp::execute(&project_root, explicit_binding)?;
        }
        Commands::Dashboard {
            watch,
            parallel,
            accept_subscription_risk,
            pty,
        } => {
            // DX-038: Check subscription risk before launching dashboard
            // PTY mode is interactive TUI — Anthropic's preferred pattern, no risk
            let pty_from_config = {
                let forge_dir = project_root.join(".forge");
                if forge_dir.exists() {
                    let state_mgr = crate::core::state::StateManager::new(&forge_dir);
                    state_mgr
                        .load()
                        .map(|s| s.dashboard_mode == "pty")
                        .unwrap_or(false)
                } else {
                    false
                }
            };
            let is_pty = pty || pty_from_config;
            if !accept_subscription_risk && !is_pty {
                let forge_dir = project_root.join(".forge");
                if forge_dir.exists() {
                    let state_mgr = crate::core::state::StateManager::new(&forge_dir);
                    if let Ok(state) = state_mgr.load() {
                        let has_claude_sub = state
                            .agent_auth
                            .get("claude")
                            .map(|v| v == "subscription")
                            .unwrap_or(true);
                        if has_claude_sub {
                            println!();
                            println!(
                                "  \u{26A0}\u{26A0}\u{26A0}  SUBSCRIPTION RISK DETECTED  \u{26A0}\u{26A0}\u{26A0}"
                            );
                            println!();
                            println!("  Claude is configured to use subscription auth.");
                            println!("  Anthropic ACTIVELY BLOCKS third-party CLI orchestration");
                            println!("  and has BANNED accounts for this pattern.");
                            println!();
                            println!("  Options:");
                            println!("    1. Switch to API mode (RECOMMENDED):");
                            println!("       forge config claude.auth api");
                            println!();
                            println!("    2. Accept the risk:");
                            println!("       forge dashboard --i-accept-subscription-risk");
                            println!();
                            println!("  See: docs/research/cli-subscription-gating-analysis.md");
                            return Ok(());
                        }
                    }
                }
            }
            cli::dashboard::execute(&project_root, parallel, watch, pty).await?;
        }
        Commands::Ship { auto, dry_run } => {
            cli::ship::execute(&project_root, auto, dry_run)?;
        }
        Commands::Uninstall { force } => {
            cli::uninstall::execute(&project_root, force)?;
        }
        Commands::Config { key, value } => {
            if let Some(key) = key {
                if let Some(value) = value {
                    cli::config::execute(&project_root, &key, &value)?;
                } else {
                    anyhow::bail!(
                        "Missing value.\n\nUsage: forge config <key> <value>\n\nExamples:\n  forge config brain openai\n  forge config brain.model gpt-4.1"
                    );
                }
            } else {
                cli::config::show(&project_root)?;
            }
        }
    }

    Ok(())
}

/// Initialize tracing infrastructure. Only active when RUST_LOG is set.
/// Logs to stderr + .forge/debug.log (non-blocking file appender).
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Only activate if RUST_LOG is set — zero overhead otherwise.
    let filter = match EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(_) => return, // RUST_LOG not set — no tracing
    };

    // File appender: writes to .forge/debug.log (non-blocking)
    let forge_dir = std::env::current_dir().unwrap_or_default().join(".forge");
    std::fs::create_dir_all(&forge_dir).ok();
    let file_appender = tracing_appender::rolling::never(&forge_dir, "debug.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so the file writer stays alive for the process lifetime
    std::mem::forget(_guard);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false),
        )
        .init();
}
