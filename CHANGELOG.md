# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — `forge doctor` (W2-B phase 1, rides the v1.6.0 train)

- **`forge doctor`** — aggregates quality health, release debt, and drift into one fail-closed
  verdict. `FAIL` always exits 1; `--strict` escalates `WARN` to 1; `--json` emits the report
  under schema `forge.doctor.report.v1`. No new MCP tools (the count stays at 11) and no new
  runtime dependencies.
- **Release-debt evaluator** (`core::release_debt`) — commits-since-tag, tag↔manifest drift,
  **lockfile agreement** (`Cargo.toml`↔`Cargo.lock`, `package.json`↔`package-lock.json`), and
  **multi-surface agreement** across every file in a repo that declares a version. Checking a
  single manifest reports an internally inconsistent repo as healthy; this compares them all.
- **`forge ship` release preflight** — a `FAIL`-level release-debt verdict now blocks the ship;
  `WARN` is surfaced and allowed, since shipping is often what discharges that debt.

### Changed

- A **critical** governance finding now fails the doctor's quality dimension regardless of the
  numeric health score. The 5-dimension score is an average, so a critical defect could sit under
  a comfortable 80/100 and pass unnoticed.
- The doctor's drift dimension reports **SKIP** rather than OK when the configured brain cannot
  measure drift. The rule-based brain returns a neutral `0.0` that is indistinguishable from
  "genuinely aligned"; reporting it as a pass manufactured a green.

## [1.5.2] - 2026-07-18

Release-discipline catch-up: two fixes shipped to `main` on 2026-05-05 were never cut into a release.
No new MCP tools (the tool count stays at 11). 378 tests green.

### Fixed

- **Silent error swallows in the MCP tool layer** (`src/mcp/tools.rs`) — 8 `let _ = …` discards of
  `event_logger.log()`, `state_mgr.unlock_files()`, and `state_mgr.refresh_task_summary()` results now
  log to stderr on failure instead of dropping the error. A failed audit-log write or lock release is no
  longer invisible. CRUCIBLE Gate 5 remediation, 57-day backlog closed. (`76d790f`)

### Added

- **`scripts/nexus-health-guard.sh`** — change-detection gate and kill-switch for `ScheduleWakeup`
  NEXUS health-check loops, preventing unbounded self-rescheduling. Shipped in the same commit as the
  Gate 5 fix above. (`76d790f`)
- **`docs/rfc/0001-forge-artifact-schema-versioning.md`** — PROPOSAL (no code change) for a semver
  schema-version handshake on `.forge/state.json` and `.forge/events.jsonl`, with the consumer
  compatibility rule for forge-ui and forge-plugin, and a position on the `forge_get_health` MCP name
  collision. Closes deep-dive gaps G-05 and G-04 at the design level; implementation is a later wave.

## [1.5.1] - 2026-05-05

### Security & Dependencies

- **RUSTSEC-2026-0097 fixed** — `rand` 0.9.2 → 0.9.3 (unsoundness when a custom logger calls `rand::rng()`). PR #21.
- **ratatui** 0.29 → 0.30 — drops `paste` (RUSTSEC-2024-0436) and `lru` (RUSTSEC-2026-0002) transitive dependencies. Clears two YELLOW CI advisories. PR #19.
- **CI audit** — `RUSTSEC-2025-0119` (`number_prefix` unmaintained, upstream-blocked on indicatif) permanently ignored with 90-day re-evaluation date (2026-08-03). PR #20.

### Documentation

- **Canonical positioning** — README hero section rewritten with multi-tool orchestration framing. `Cargo.toml` description updated. `docs/canonical-positioning.md` added. PR #16.

## [1.4.1] - 2026-03-16

### Tier 2 Observability

- **Debug/trace mode** — `tracing` + `tracing-subscriber` + `tracing-appender`. Zero overhead when `RUST_LOG` unset. When set, writes to `.forge/debug.log`. `debug!()` spans at PTY spawn, task completion, event write, MCP dispatch.
- **`forge_get_events` MCP tool** — 11th tool. Parameters: `count` (default 50), optional `task_id` filter, optional `event_type` filter. Returns structured JSON with `duration_ms`/`exit_code` fields.
- **Knowledge auto-capture** — Confirmed working from v1.4.0 (no additional changes needed).

## [1.4.0] - 2026-03-15

### Observability & Logging (P1 — Launch Readiness)

Addresses CLX9 dogfood findings: skeletal logging, missing task outputs, empty knowledge directories, stale state.json during dashboard runs. Implements all 4 Tier 1 items from the observability brief.

### Added

- **Full agent output capture** (`app.rs`) — PTY mode: `pty_session.snapshot()` flushes vt100 buffer to `.forge/results/{task_id}.txt` on every task completion/failure. Piped mode: `agent_outputs` buffer written to same path. All task outputs now persisted to disk.
- **Structured event fields** (`event.rs`) — `duration_ms: Option<u64>` and `exit_code: Option<i32>` as first-class fields on `ForgeEvent`. Task duration tracked from dispatch to completion using `Instant::now()`.
- **Real-time phase tracking** (`state.rs`, `app.rs`) — `current_phase: Option<String>` added to `ForgeState`. State.json updated at every Build→Verify→Complete phase transition.
- **Cross-agent verification** (`task.rs`, `app.rs`) — `verified_by: Option<AgentType>` and `verified_at: Option<DateTime<Utc>>` fields on `Task`. Auto-populated when V-xxx verify tasks complete, linking verification agent to parent build task.
- **Rich event logging** (`app.rs`) — PTY result capture and knowledge auto-extraction on task completion.

### Changed

- Stargate Auto-Approve Contract documented in CLAUDE.md — all 3 adapters (Claude, Codex, Gemini) now marked as having unconditional auto-approve.

## [1.3.2] - 2026-03-14

### Codex PTY Crash Fix (P0)

Resolves Codex CLI crashing immediately (exit 1) when spawned inside Forge Stargate dashboard PTY panes. Claude PTY was unaffected.

### Fixed

- **Codex PTY launch mode** (`codex.rs`) — Use `codex exec --full-auto --skip-git-repo-check "prompt"` instead of bare `codex`. Root cause: Codex's `resolve_prompt()` calls `exit(1)` when `stdin.is_terminal() == true` with no prompt argument. Passing prompt as CLI arg bypasses all 8 of Codex's validation gates.
- **`--no-alt-screen` flag** (`codex.rs`) — Prevents rendering conflicts in nested terminal contexts (ratatui TUI → vt100 → Codex TUI). This flag exists specifically for embedded/multiplexer contexts.
- **Environment variable leak** (`app.rs`) — Both PTY spawn paths now call `env_remove()` for vars adapters explicitly removed. Previously, `None` values from `get_envs()` were silently skipped, meaning API keys (e.g., `OPENAI_API_KEY` in subscription mode) leaked into PTY processes.
- **Exit code masking** (`pty_session.rs`) — Preserve real exit codes via `status.exit_code()` instead of `if success { 0 } else { 1 }`. Exit codes 127, 2, 126 etc. now visible for debugging.
- **PTY crash output capture** (`app.rs`) — When a PTY process exits with failure, last visible output from vt100 parser is captured and logged as an event before cleanup.

### Dependencies

- Bump quinn-proto from 0.11.13 to 0.11.14

## [1.3.1] - 2026-03-09

### Fixed

- **CI formatting** — Apply `cargo fmt` to state.rs and governance.rs test code (formatting drift from CRUCIBLE mutation testing remediation).
- **README walkthrough** — Added guided Quick Demo section for new users.

## [1.3.0] - 2026-03-08

### The Stargate Era

Forge 1.3 ships the three headline features from the v1.3.0 roadmap: **Stargate** embeds full interactive PTY agent panes directly in the dashboard (no more watching text scroll — you see Claude's actual TUI), **Builder Mode** gives agents the ability to signal their own completion via signal files, and **Quality Gates** (DX-052) introduces an 8-check automated gate engine with A–F letter grading and an optional Playwright smoke gate. Three-Tier Validation (DX-051) extends the T→V→U lifecycle to include human UAT as a first-class phase. Subscription safety rails (DX-033–038) add pacing, rate-limit detection, exponential backoff, provider rotation, quota monitoring, and risk warnings. The test suite grew from 200 to 362 with a full CRUCIBLE Gate 6 mutation audit (governance.rs 88.1%, state.rs 100%).

### Added

- **DX-024 Stargate — Embedded Interactive PTY Agent Panes** — `forge dashboard` now spawns Claude, Codex, and Gemini in native PTY sessions inside the dashboard. Each agent pane is a full interactive terminal, not a pipe. Agents see their own TUI (Claude's interactive mode, Codex's REPL, Gemini's interface). Keyboard input routes to the focused pane.
- **vt100 terminal emulator** — replaced the bespoke `AnsiLineCollector` with the `vt100` crate for correct full-screen terminal emulation inside PTY panes. Cursor positioning, scrolling, alternate-screen buffers, and ANSI sequences all render correctly.
- **Adaptive ready-pattern detection + PTY resize** — PTY panes detect when an agent's TUI is ready to receive input via configurable ready-pattern polling (200ms, 300ms grace). Panes resize automatically on expand/collapse.
- **Full TUI mode for Codex and Gemini** — Codex and Gemini now launch their native TUIs inside Stargate panes (not just Claude). All three agents supported in interactive PTY mode.
- **DX-050 Builder Mode** — agents signal task completion by writing a signal file (`.forge/signals/T-xxx.complete`). Dashboard detects the file and transitions the task without polling the process. Adds idle TUI spawn and color-coded pane headers per agent type.
- **`c` key cycles agent assignment** — on the task board, pressing `c` cycles the selected task's agent assignment (Claude → Codex → Gemini → Claude). Reassign without leaving the dashboard.
- **DX-051 Three-Tier Validation — T→V→U pipeline** — Human UAT (`forge uat`) is now a first-class phase. After BUILD→VERIFY completes, dashboard auto-transitions to UAT phase. `forge uat` launch auto-generates U-xxx acceptance tasks from completed build tasks.
- **DX-052 Quality Gates — 8-check automated gate engine** — `forge verify` runs 8 quality gates: TypeScript compilation, test coverage, lint, security audit, dependency freshness, documentation, commit hygiene, build size. Each gate produces an A–F letter grade with configurable thresholds. Gate results feed into the governance health score.
- **DX-052 Quality Gates — Playwright smoke gate** — optional ninth gate runs a Playwright headless browser check against the project's UI. Catches rendering regressions invisible to unit tests.
- **DX-033/034/035/036 — Subscription safety rails** — four new protections for Pro/Max subscription users: token-bucket pacing (prevents burst), rate-limit detection (429 → automatic backoff), exponential backoff with jitter (10s→30s→60s→120s, max 5 retries), and provider rotation (fails over Claude→Codex→Gemini on sustained errors). All four ported to TUI dashboard mode.
- **DX-037 Quota monitoring** — dashboard header shows remaining quota estimate. Warning triggered at configurable threshold.
- **DX-038 Subscription risk warning** — banner fires when Codex is running in subscription mode with active tasks that could exhaust quota. Lets the operator intervene before hitting the cap.
- **CRUCIBLE Gate 6 remediation** — full mutation testing audit (cargo-mutants). governance.rs: 0% → 88.1% (52/59 viable mutations caught). state.rs: 34.8% → 100% (23/23 caught). 69 new value-asserting tests replace hollow `is_some()`/`is_empty()` existence checks.
- **ForgeSentinel** — CODEOWNERS, PR template, and SECURITY.md for open-source hardening.
- **CodeQL security scanning** — GitHub Actions workflow runs Rust static analysis on every push.
- **CI failure notifications** — failed builds automatically open a GitHub Issue via `failed-build-issue-action`.
- **GitHub Sponsors** — funding link added to repository.

### Fixed

- **Stargate PTY bugfixes** — backspace rendering in agent panes, attach/detach/expand state machine, PTY completion detection false-positives (pattern-disappeared guard added).
- **DX-033–036 TUI port** — subscription pacing, rate-limit detection, backoff, and provider rotation were headless-only; ported all four to the live TUI dashboard execution path.
- **Claude adapter `--verbose` flag** — `stream-json` mode requires `--verbose` for NDJSON output; missing flag caused buffered output instead of live streaming.
- **`forge init` next-steps ordering** — brain config step now appears before `plan --generate` in the post-init guidance (was reversed, confusing new users).

### Changed

- **362 tests** (340 unit + 10 CLI + 12 MCP), up from 200. Includes 69 new mutation-resistant governance and state tests.
- **Terminology**: "coordination" replaced with "orchestration" across all user-facing CLI output, README, and docs.
- **CI matrix**: Windows dropped from the test matrix — PTY allocation and `Instant`-based timing tests are not compatible with Windows CI runners. Linux + macOS remain.
- **MCP tool count**: corrected to 10 in all docs (was documented as 9 in several places).
- **README**: full public-launch rewrite with one-product framing, L2 spark mark logo, and accurate binary stats (v1.3.0, 362 tests, 10 MCP tools, 4 MB).

## [1.2.0] - 2026-02-13

### The UAT Commander

Forge 1.2 replaces the bare-bones `forge uat` REPL with a full ratatui TUI for human acceptance testing, adds live agent streaming to the dashboard, and fixes a UTF-8 crash.

### Added
- **DX-032: Standalone UAT TUI** — `forge uat` now launches a dedicated ratatui interface with task selector (filters out V-xxx verify subtasks), focused acceptance criteria view per task, finding capture with auto-classification, and pass/fail marking via keyboard shortcuts
- **DX-032: Inline UAT capture** — `forge uat "description"` one-shot mode captures a finding without opening the TUI
- **DX-029: Live agent streaming** — Claude adapter switched from `--output-format text` (buffered) to `stream-json` (real-time NDJSON). Dashboard agent panes now show live activity: `[Read] src/main.rs`, `[Bash] npm test`, `[Edit] src/foo.ts`
- **DX-030: Project name in dashboard header** — title now shows `FORGE DASHBOARD — voice-jib-jab — BUILD (12/17)` instead of generic header
- **DX-031: Freeze completion timer** — elapsed time stops counting when all tasks complete (was ticking forever)

### Fixed
- **UTF-8 truncation panic** — all 3 `truncate` functions (status.rs, plan.rs, app.rs) crashed on multi-byte characters like em dash. Fixed with `.chars().count()` + `.chars().take(n)` instead of byte slicing

### Changed
- 200 tests passing (178 unit + 10 CLI + 12 MCP), up from 179
- New files: `src/tui/uat_app.rs`, `src/tui/uat_ui.rs`

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

[Unreleased]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.3.0...HEAD
[1.3.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.3.0...v1.0.0
[0.3.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/nxtg-ai/forge-orchestrator/releases/tag/v0.1.0
