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
    // Load .env file if present (for OPENAI_API_KEY etc.)
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let project_root = PathBuf::from(&cli.project)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&cli.project));

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
                    anyhow::bail!("Missing value. Usage: forge config <key> <value>");
                }
            } else {
                cli::config::show(&project_root)?;
            }
        }
    }

    Ok(())
}
