pub mod config;
pub mod dashboard;
pub mod init;
pub mod mcp;
pub mod plan;
pub mod run;
pub mod start;
pub mod status;
pub mod sync;
pub mod uat;
pub mod verify;

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

        /// Generate fix tasks from UAT findings (.forge/findings/)
        #[arg(long)]
        from_findings: bool,
    },

    /// Show orchestration status — task board, agent activity, governance
    Status {
        /// Number of recent events to show
        #[arg(short, long, default_value = "5")]
        events: usize,
    },

    /// Execute tasks headlessly. Omit --task/--agent for autonomous parallel mode.
    Run {
        /// Task ID (e.g., T-001). Omit to run ALL tasks autonomously.
        #[arg(short, long)]
        task: Option<String>,

        /// Agent to execute the task (claude, codex, gemini). Omit for auto-assign.
        #[arg(short, long)]
        agent: Option<String>,

        /// Maximum number of parallel agent tasks (autonomous mode)
        #[arg(short, long, default_value = "3")]
        parallel: usize,

        /// Show what would run without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// Start autonomous orchestration — run all tasks with auto-claim/complete
    Start {
        /// Only run tasks for a specific agent (claude, codex, gemini)
        #[arg(short, long)]
        agent: Option<String>,

        /// CEO Mode: loop until all tasks complete (re-runs after each pass)
        #[arg(short, long, alias = "ceo")]
        r#loop: bool,

        /// Accept subscription risk for providers that may ban automated usage
        #[arg(long = "i-accept-subscription-risk", default_value_t = false)]
        accept_subscription_risk: bool,
    },

    /// Reconcile state — update summaries, render adapter configs, check governance
    Sync,

    /// Start the MCP server (stdio transport) — AI tools connect to query/update state
    Mcp,

    /// Live TUI dashboard — task board, agent output, event log
    Dashboard {
        /// Watch mode: display tasks without auto-executing
        #[arg(short, long)]
        watch: bool,

        /// Maximum number of parallel agent tasks
        #[arg(short, long, default_value = "3")]
        parallel: usize,

        /// Accept subscription risk for providers that may ban automated usage
        #[arg(long = "i-accept-subscription-risk", default_value_t = false)]
        accept_subscription_risk: bool,

        /// Enable Stargate PTY mode — agents render with full terminal colors and interactivity
        #[arg(long)]
        pty: bool,
    },

    /// Generate verify subtasks for completed build tasks
    Verify,

    /// Interactive UAT — describe issues naturally, capture findings
    Uat {
        /// Quick capture: describe a finding inline without opening TUI
        #[arg()]
        finding: Option<String>,
    },

    /// Get or set configuration values
    Config {
        /// Config key to set (e.g., "brain", "brain.model"). Omit to show current config.
        #[arg()]
        key: Option<String>,

        /// Value to set
        #[arg()]
        value: Option<String>,
    },
}
