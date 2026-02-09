mod adapters;
mod brain;
mod cli;
mod core;
mod detect;

use clap::Parser;
use cli::{Cli, Commands};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let project_root = PathBuf::from(&cli.project)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&cli.project));

    match cli.command {
        Commands::Init { name } => {
            cli::init::execute(&project_root, name)?;
        }
        Commands::Plan => {
            cli::plan::execute(&project_root)?;
        }
        Commands::Status { events } => {
            cli::status::execute(&project_root, events)?;
        }
        Commands::Run { task, agent } => {
            cli::run::execute(&project_root, &task, &agent)?;
        }
    }

    Ok(())
}
