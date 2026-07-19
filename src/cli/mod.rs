/// Configuration management (get/set brain, model, etc.).
pub mod config;
/// Live TUI dashboard with task board, agent output panes, and event log.
pub mod dashboard;
/// Aggregate health verdict — quality gates, release debt, and drift in one exit code.
pub mod doctor;
/// Project initialization — scaffolds `.forge/` directory and detects AI tools.
pub mod init;
/// MCP server startup (stdio JSON-RPC 2.0 transport).
pub mod mcp;
/// Master plan generation and display from SPEC.md or UAT findings.
pub mod plan;
/// Declarative tmux pod management (cosmux parity surface).
pub mod pod;
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
        /// Show the fleet context-budget HUD (per-pane ctx% + PREP/COMPACT/STOP) instead
        #[arg(long)]
        budget: bool,
        /// With --budget: emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// With --budget: include panes with no ctx gauge (n/a)
        #[arg(long)]
        all: bool,
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

    /// Aggregate health verdict — quality, release debt, and drift in one fail-closed exit code
    Doctor {
        /// Escalate WARN to a non-zero exit (FAIL always exits non-zero)
        #[arg(long)]
        strict: bool,

        /// Emit the report as JSON (schema `forge.doctor.report.v1`)
        #[arg(long)]
        json: bool,
    },

    /// Declarative tmux pod management — start, stop, and recover multi-pane agent pods
    Pod {
        #[command(subcommand)]
        verb: PodVerb,
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

/// `forge pod` verbs. Parity surface with cosmux v0.4.2: 11 public + 2 hidden.
#[derive(Subcommand, Debug)]
pub enum PodVerb {
    /// Spawn a pod (creates the tmux session detached)
    Start {
        /// Pod name or path to a pod YAML
        name: String,
        /// Replace an existing session of the same name
        #[arg(long)]
        force: bool,
        /// Print the attach command after starting
        #[arg(long)]
        attach: bool,
    },

    /// Kill a pod's tmux session
    Stop {
        /// Pod name
        name: String,
    },

    /// List running tmux sessions
    List,

    /// Validate a pod YAML config (no side effects)
    Validate {
        /// Pod name or path to a pod YAML
        name: String,
    },

    /// Print the resolved pod config (after template merge)
    Show {
        /// Pod name or path to a pod YAML
        name: String,
    },

    /// Print HUD state.json path + contents
    ///
    /// `hud` is an alias, not a separate verb — cosmux registers it that way and users type both.
    #[command(alias = "hud")]
    State,

    /// List forge-managed pods only (vs `list`, which shows all tmux sessions)
    Ps,

    /// Garbage-collect state.json: drop entries whose tmux session no longer exists
    Gc,

    /// Reload a pod (stop + start). Re-reads YAML; loses agent conversation context.
    Reload {
        /// Pod name
        name: String,
        /// Replace an existing session of the same name
        #[arg(long)]
        force: bool,
    },

    /// Print shell completion script (bash | zsh | fish | powershell | elvish)
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
    },

    /// Verify every heartbeat target has a matching pod window (exit 2 when uncovered)
    Preflight {
        /// Pod name or path to check (omit to check every session referenced by heartbeat)
        pod: Option<String>,
        /// Path to heartbeat script to parse (default: auto-detect by hostname)
        #[arg(long)]
        against: Option<String>,
    },

    /// Adopt the cosmux store: install the shim, rebind live hooks to forge, become the writer
    Adopt {
        /// Roll the journal deterministically back to unadopted (recovery from an inconsistent state)
        #[arg(long, conflicts_with = "abort")]
        repair: bool,
        /// Reverse an interrupted adoption back to unadopted
        #[arg(long)]
        abort: bool,
    },

    /// Roll back adoption: restore cosmux hooks, remove the shim, cosmux writes again
    Unadopt,

    /// Internal: re-spawn dead panes for a session (invoked by the tmux pane-died hook)
    #[command(name = "_pane-recover", hide = true)]
    PaneRecover {
        /// tmux session name
        session: String,
    },

    /// Internal: run after_detach hooks (invoked by the tmux client-detached hook)
    #[command(name = "_after-detach", hide = true)]
    AfterDetach {
        /// tmux session name
        session: String,
    },
}
