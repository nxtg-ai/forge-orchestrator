# CLAUDE.md — Forge Orchestrator

Forge Orchestrator is the public Rust coordination engine for AI-powered development workflows. It plans work, coordinates supported agent tools, manages task/file state, exposes MCP interfaces, and provides terminal-oriented execution surfaces.

## Quick Reference

```bash
cargo build --release
cargo test
cargo clippy -- -W clippy::all
cargo run -- init
cargo run -- plan --generate
cargo run -- mcp
cargo run -- status
```

## Architecture

```text
src/
├── cli/       # command handlers
├── core/      # deterministic project/task/state logic
├── adapters/  # supported agent-tool adapters
├── mcp/       # stdio MCP server
├── tui/       # terminal interfaces
├── brain/     # optional planning/reasoning backends
└── detect/    # local tool detection
```

## Engineering Rules

- Keep deterministic state transitions observable and testable.
- Preserve file-locking and task-lifecycle guarantees during parallel execution.
- Keep MCP interfaces backward-compatible within a released version line.
- Treat PTY automation and unattended tool execution as security-sensitive behavior; verify actual tool contracts rather than relying on help text alone.
- Use synthetic fixtures for multi-project and multi-agent tests.
- Run formatting, clippy, and the full test suite before release changes.

## Public / Private Boundary

This is a public repository. Do not commit private portfolio state, internal directives or handoffs, production fleet topology, organization-internal session names, private repository paths, local machine topology, internal network endpoints, private cross-project memory/retrieval wiring, credentials, or generated audit records from private repositories.

Public tests must use synthetic workspace names and paths. Organization-internal runtime context must be injected outside this repository.

## Release Discipline

When changing a public release, keep Cargo metadata, changelog entries, tags, binaries, and GitHub release artifacts synchronized. Do not bypass failing test, lint, or release gates.
