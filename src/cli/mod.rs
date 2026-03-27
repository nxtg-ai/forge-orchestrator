/// Configuration management (get/set brain, model, etc.).
pub mod config;
/// Live TUI dashboard with task board, agent output panes, and event log.
pub mod dashboard;
/// Project initialization — scaffolds `.forge/` directory and detects AI tools.
pub mod init;
/// MCP server startup (stdio JSON-RPC 2.0 transport).
pub mod mcp;
/// Master plan generation and display from SPEC.md or UAT findings.
pub mod plan;
/// Headless task execution — single-task or autonomous parallel mode.
pub mod run;
/// Ship phase — changelog generation, artifact archival, and state cleanup.
pub mod ship;
/// Autonomous orchestration loop — auto-claim, execute, and complete tasks.
pub mod start;
/// Status display — task board, agent activity, and governance summary.
pub mod status;
/// State reconciliation — update summaries, render adapter configs, check governance.
pub mod sync;
/// Interactive UAT — capture user acceptance findings via TUI or inline.
pub mod uat;
/// Uninstall Forge — remove binary and optionally project data.
pub mod uninstall;
/// Generate verify subtasks for completed build tasks.
pub mod verify;

use clap::{Parser, Subcommand};

/// Top-level CLI definition parsed by clap.
///
/// Entry point for all `forge` subcommands. The `--project` flag allows
/// targeting a specific project root directory.
#[derive(Parser)]
#[command(
    name = "forge",
    version,
    about = "NXTG-Forge Orchestrator — Universal orchestration engine for AI-powered development",
    long_about = "Forge orchestrates Claude Code, Codex CLI, Gemini CLI, and future AI tools \
                  as a coordinated team with governance, knowledge capture, and conflict prevention."
)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Path to the project root (defaults to current directory)
    #[arg(long, global = true, default_value = ".")]
    pub project: String,
}

/// All available `forge` subcommands.
///
/// Each variant maps to a CLI subcommand (e.g., `forge init`, `forge plan`).
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

    /// Ship phase — generate changelog, archive artifacts, clean state for next cycle
    Ship {
        /// Auto-approve all steps (non-interactive)
        #[arg(long)]
        auto: bool,

        /// Show what would happen without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// Uninstall Forge — remove binary and optionally project data
    Uninstall {
        /// Also remove .forge/ project data and ~/.forge/ global config
        #[arg(long)]
        force: bool,
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
