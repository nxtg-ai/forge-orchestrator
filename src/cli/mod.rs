pub mod init;
pub mod mcp;
pub mod plan;
pub mod run;
pub mod status;
pub mod sync;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "forge",
    version,
    about = "NXTG-Forge Orchestrator — Universal coordination engine for AI-powered development",
    long_about = "Forge orchestrates Claude Code, Codex CLI, Gemini CLI, and future AI tools \
                  as a coordinated team with governance, knowledge capture, and conflict prevention."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Path to the project root (defaults to current directory)
    #[arg(long, global = true, default_value = ".")]
    pub project: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize Forge in a project — scaffold .forge/, detect AI tools
    Init {
        /// Project name (auto-detected from directory if not specified)
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Show or generate the master plan
    Plan {
        /// Generate plan from SPEC.md (CEO Mode)
        #[arg(short, long)]
        generate: bool,

        /// Path to spec file (defaults to SPEC.md in project root)
        #[arg(short, long)]
        spec: Option<String>,
    },

    /// Show orchestration status — task board, agent activity, governance
    Status {
        /// Number of recent events to show
        #[arg(short, long, default_value = "5")]
        events: usize,
    },

    /// Execute a task headlessly on a specific AI tool
    Run {
        /// Task ID (e.g., T-001)
        #[arg(short, long)]
        task: String,

        /// Agent to execute the task (claude, codex, gemini)
        #[arg(short, long)]
        agent: String,
    },

    /// Reconcile state — update summaries, render adapter configs, check governance
    Sync,

    /// Start the MCP server (stdio transport) — AI tools connect to query/update state
    Mcp,
}
