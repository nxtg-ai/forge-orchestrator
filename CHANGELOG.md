# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.0] - 2026-02-11

### The Verifier

Forge 1.1 adds a complete verification lifecycle. After BUILD, the dashboard auto-transitions to VERIFY phase, generating verify subtasks for every build task. Failed verifications spawn fix+re-verify pairs (up to 3 retries). Human UAT captures findings interactively, and `forge plan --from-findings` converts them into fix tasks.

### Added
- **Phase lifecycle**: Build -> Verify -> Complete auto-transition in the TUI dashboard
- **`forge verify`**: Generate verify subtasks for completed build tasks (V-NNN IDs)
- **`forge uat`**: Interactive UAT REPL with keyword-based severity classification (Critical/High/Medium/Low/Positive)
- **`forge plan --from-findings`**: Convert UAT findings into fix tasks with phase=Fix, severity-based priority (P0-P3)
- **TaskPhase enum**: `Build`, `Verify`, `Fix` — lifecycle phase tracking per task
- **Task model extensions**: `parent_task`, `phase`, `retry_count` fields with backward-compatible serde
- **Finding model**: `FindingSeverity`, `FindingType`, `Finding` struct, `FindingManager` with JSON persistence
- **Verify/fix loop**: Failed verify tasks auto-generate fix + re-verify pairs (max 3 retries)
- **Dashboard phase indicator**: Title shows `FORGE DASHBOARD — BUILD (12/17)` with phase-specific progress
- **Hierarchical task display**: Subtasks indented under parents in dashboard and status views
- **Phase column in status**: `forge status` shows Phase (build/verify/fix) for each task

### Changed
- 179 tests passing (157 unit + 10 CLI + 12 MCP), up from 139
- Dashboard completion detection is now phase-aware (only completes after Verify→Complete transition)

## [1.0.0] - 2026-02-11

### The Autonomous Builder

Forge 1.0 is the first release where you can type `forge init && forge plan --generate && forge dashboard` and walk away. The dashboard orchestrates Claude, Codex, and Gemini in parallel, handles rate limits, commits per task, and stays open when done so you can review results.

### Added
- **DX-018: Rate Limit Backoff** — exponential backoff (10s→30s→60s→120s) with jitter when agents hit API rate limits. Per-agent tracking, max 5 retries, staggered restarts
- **DX-020: Key Legend** — footer bar shows keyboard shortcuts (`q:Quit | Tab:Focus | ↑↓:Nav | Enter:Detail | r:Retry | s:Shell | ?:Help`)
- **DX-021: Orphan Task Cleanup** — on startup, resets stale in-progress tasks to pending. On quit, resets all running tasks
- **DX-022: No Auto-Exit** — dashboard stays open on completion with summary banner. Press `q` to exit
- **DX-023: Interactive Terminal Panes** — scrollable agent output with `↑↓` when focused, `Tab` to cycle focus, visual border highlight
- **DX-025: Monotonic Task IDs** — `plan --generate` appends new tasks from highest existing ID. Never overwrites completed tasks. Adds `plan_version` field
- **DX-026: Priority Key Handling** — key events drain first via `try_recv()` loop before processing agent output. 100ms poll rate (was 250ms). Keys register instantly even under heavy agent output
- **DX-027: Shell Panes** — press `s` or `+` to spawn a `$SHELL` in the Summary pane. Type commands while agents work. `Ctrl+D` to close
- **DX-028: Git Auto-Commit** — after each task completes, runs `git add -A && git commit` with conventional format (`feat(T-007): Task title`). Configurable via `forge config git.auto_commit true|false`
- **DX-010: Full Task Table in Status** — `forge status` shows complete task board with ID, status (Ready/Blocked/Running/Done/Failed), agent, type, and color-coded dependencies
- **DX-011: Headless Autonomous Mode** — `forge run` (no args) runs all tasks in parallel headlessly. `--parallel N` limits concurrency, `--dry-run` shows execution plan without running. Same dependency-aware scheduling as dashboard
- **DX-019: Gemini Adapter Fix** — added `-p` flag for headless mode, `--yolo` + `--sandbox=false` in yolo permissions

### Changed
- 139 tests passing (117 unit + 10 CLI + 12 MCP), up from 71
- Source lines: ~10,650 (up from ~5,000)
- Binary size: 3.7 MB
- `--task` and `--agent` are now optional on `forge run`

## [0.3.0] - 2026-02-11

### Added
- **DX-014: TUI Dashboard** — `forge dashboard` launches a live terminal UI for multi-agent orchestration
  - Task board table with status icons, colors, and progress indicators (top 30%)
  - 2x2 agent pane grid with live streaming output from spawned processes (middle 50%)
  - Event log with timestamped entries (bottom 20%)
  - **Live execution**: spawns agent processes via `tokio::process::Command`, pipes stdout/stderr line-by-line to panes via `mpsc` channels
  - **Auto-scheduling**: dependency-aware task scheduling — when a task completes, unblocked tasks auto-start (up to `--parallel N`, default 3)
  - Keyboard navigation: `q`/`Esc` quit, `Tab` cycle focus, `↑↓` navigate tasks, `Enter` view detail, `r` retry failed
  - Panic hook restores terminal (disables raw mode + leaves alternate screen)
  - Ring buffer caps agent output at 200 lines per pane
  - Works over SSH (keyboard-only, no mouse required)
- **CLI flags**: `forge dashboard --watch` (read-only), `forge dashboard --parallel 4` (max concurrent agents)
- New dependencies: `ratatui = "0.29"`, `crossterm = "0.28"`
- 12 new TUI tests (unit tests for scheduling, rendering via TestBackend)
- `Eq` and `Hash` derives on `AgentType` for HashMap support

### Changed
- **71 tests** passing (49 unit + 10 integration + 12 MCP), up from 59

## [0.2.2] - 2026-02-11

### Added
- **DX-009: Spinner / progress indicators** — phased spinners during plan generation and task execution using `indicatif` crate
  - 5 phases in `plan --generate`: spec loading, codebase scan, task decomposition, agent assignment, disk write
  - Spinner during headless `run --task` execution with success/failure finish message
  - Removed `eprintln!` debug noise from OpenAI brain that clashed with spinner output
- **DX-017: Codebase-aware plan generation** — spec vs reality diff before task generation
  - New `scan_codebase()` in plan.rs: walks source dirs with `walkdir`, extracts file paths + line counts + export signatures
  - Token-budgeted to ~4000 tokens, 3 levels deep, skips node_modules/target/.git/dist/build
  - Combined spec + codebase inventory sent to brain with gap-only instructions
  - OpenAI brain system prompt updated to skip tasks for features that already exist
  - Rule-based brain creates "review" tasks instead of "implement" when matching files found
  - Greenfield projects (no source files) behave identically to before
- New dependencies: `indicatif = "0.17"`, `walkdir = "2"`

## [0.2.1] - 2026-02-11

### Added
- **DX-016: Smart Claude Adapter** — task-type-aware invocation replaces blanket `--dangerously-skip-permissions`
  - `task_type` field on Task struct (`design`, `implement`, `review`, `test`, `document`)
  - OpenAI brain classifies task types during `plan --generate`
  - Rule-based brain uses keyword heuristics for task type classification
  - Claude adapter scopes `--allowedTools` per task type (e.g., review tasks are read-only)
  - Task-type-aware `--max-turns` (design: 30, review/test: 20, others: default)
  - Type column in plan table and `plan.md` output

### Changed
- `design` tasks get full permissions + architecture-focused prompts
- `review`/`test` tasks restricted to `Read,Glob,Grep,Bash` (no file mutations)
- `document` tasks get `Write,Edit,Read,Glob,Grep` (no Bash)
- `implement` tasks get `Write,Edit,Read,Glob,Grep,Bash` (focused toolset)
- Unknown/legacy tasks fall back to `--dangerously-skip-permissions` for backward compat

## [0.2.0] - 2026-02-11

### Added
- **Per-agent auth config**: `forge config claude.auth subscription|api` — controls whether API keys are passed to CLI subprocesses or stripped (defaults to `subscription` for Pro/Max users)
- **YOLO permissions mode**: `forge config claude.permissions yolo` — full autonomy with `--dangerously-skip-permissions` for Claude, `--full-auto` for Codex, `--sandbox=false` for Gemini
- **Async adapter execution**: Adapters now use `tokio::process::Command` via `execute_command_async()` for non-blocking agent execution
- **`build_command()` trait method**: Adapters expose a `build_command()` that returns `std::process::Command`, decoupling command construction from execution
- **CEO Mode autonomous loop**: `forge start --ceo` retries failed tasks across multiple passes with 30s cooldown between passes
- **Parallel multi-agent execution**: `run_parallel()` spawns one tokio task per agent type, each managing its own task queue
- **Transient error detection**: Retries on "credit balance too low", rate limits, timeouts, and 500 errors
- DX backlog document (`DX-BACKLOG.md`) from live dogfood sessions

### Fixed
- **DX-001**: Init now globs all `*.md` + `docs/*.md` files (was hardcoded to 4 files)
- **DX-002**: Init scaffolds `governance.json` during project setup
- **DX-003**: `plan --generate` reads project context and calls AI brain (was static template)
- **DX-004/005**: Plan generates real tasks to `.forge/tasks/` (no more phantom T-001)
- **DX-006**: Config output shows usage hints
- **DX-007**: Better error messages for positional config syntax
- **DX-008**: Brain selection now affects plan generation output
- **DX-012**: `ANTHROPIC_API_KEY` no longer leaks to Claude CLI subprocess (uses `.env_remove()`)
- Resolve all P1-P3 UAT friction points (5 fixes)

### Changed
- `ToolAdapter` trait: `execute_headless()` now has default implementation using `build_command()` + `process_output()`
- Adapter trait requires `auth_mode` and `permissions` parameters
- 59 tests passing (37 unit + 10 CLI + 12 MCP)

## [0.1.2] - 2026-02-09

### Added
- `forge start` command for fully autonomous multi-agent orchestration
- `forge start --loop` / `--ceo` for CEO Mode (zero-human-in-the-loop)
- Retry logic with progress bar and summary report in `forge start`
- Smart dependency timeout — waits while other agents make progress
- Human walkthrough guide (`docs/human-walkthrough.md`)

### Fixed
- Add `--skip-git-repo-check` to Codex adapter
- Add `500` and `api_error` to transient error retry patterns for OpenAI

## [0.1.1] - 2026-02-08

### Fixed
- Critical DX fixes: OpenAI brain API integration, all adapter spawn commands, tool auto-detection
- Codex, Claude, and Gemini adapters now produce correct shell commands
- OpenAI brain properly reads API key from `.env` and `~/.forge/.env`
- Tool detection works across Linux, macOS, and Windows

## [0.1.0] - 2026-02-08

### Added
- **Core engine**: TaskManager, StateManager, EventLogger, PlanManager, KnowledgeManager, GovernanceChecker
- **CLI commands**: `forge init`, `forge plan --generate`, `forge status`, `forge run`, `forge sync`, `forge config`
- **MCP server**: 9 JSON-RPC 2.0 tools for real-time AI-tool integration (`forge mcp`)
- **Pluggable brain**: RuleBasedBrain (free heuristic) and OpenAIBrain (gpt-4.1)
- **Adapters**: Claude Code, Codex CLI, Gemini CLI — headless task execution
- **Knowledge flywheel**: Capture, auto-classify, search, and SKILL.md generation
- **Governance**: 5-dimension health checks (documentation, architecture, task health, knowledge, drift)
- **File locking**: Automatic conflict prevention when agents claim tasks
- **Drift detection**: Vision alignment scoring via ForgeBrain against SPEC.md
- CI/CD pipeline with GitHub Actions, install script, and Windows support
- Animated SVG banner for README
- 51 tests (30 unit + 9 CLI + 12 MCP integration)

[Unreleased]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.3.0...v1.0.0
[0.3.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/nxtg-ai/forge-orchestrator/releases/tag/v0.1.0
