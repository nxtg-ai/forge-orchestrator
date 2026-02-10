# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Resolve all P1-P3 UAT friction points (5 fixes) — OpenAI timeout doubled (120s), MCP project switching via `forge_set_project`, knowledge search test coverage, plan template quality, codex adapter `--skip-git-repo-check`

### Added
- 4 new knowledge search tests: case insensitivity, tag search, empty base, content-only match (54 → 58 tests)
- Ecosystem cross-references in README linking to forge-plugin and forge dashboard repos

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

[Unreleased]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nxtg-ai/forge-orchestrator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/nxtg-ai/forge-orchestrator/releases/tag/v0.1.0
