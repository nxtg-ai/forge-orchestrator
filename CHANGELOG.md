# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/nxtg-ai/forge-orchestrator/releases/tag/v0.1.0
