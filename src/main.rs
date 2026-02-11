#![allow(dead_code)] // Phase 1: many trait methods designed for future phases

mod adapters;
mod brain;
mod cli;
mod core;
mod detect;
mod mcp;

use clap::Parser;
use cli::{Cli, Commands};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // Load .env from CWD first (default dotenvy behavior)
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let project_root = PathBuf::from(&cli.project)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&cli.project));

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
        Commands::Plan { generate, spec } => {
            cli::plan::execute(&project_root, generate, spec)?;
        }
        Commands::Status { events } => {
            cli::status::execute(&project_root, events)?;
        }
        Commands::Run { task, agent } => {
            cli::run::execute(&project_root, &task, &agent)?;
        }
        Commands::Start { agent, r#loop } => {
            if r#loop {
                cli::start::execute_loop(&project_root, agent.as_deref())?;
            } else {
                cli::start::execute(&project_root, agent.as_deref())?;
            }
        }
        Commands::Sync => {
            cli::sync::execute(&project_root)?;
        }
        Commands::Mcp => {
            cli::mcp::execute(&project_root)?;
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
