# Canonical Positioning — forge-orchestrator

> Source of truth for all surfaces: crates.io description, lib.rs, GitHub README hero,
> forge.nxtg.ai /orchestrator page. Pull from this file — do not maintain copies.
>
> **Live numbers pulled**: 2026-05-03 | v1.5.0 | binary built from clean main

---

## Register 1 — One-liner (≤100 chars, Cargo.toml description, crates.io card)

```
Orchestrate Claude Code, Codex CLI, and Gemini CLI on shared repos — single Rust binary, zero deps.
```

---

## Register 2 — Elevator (2 sentences, social / PR intro)

Running multiple AI coding tools on the same repo without coordination guarantees conflicts: one tool
refactors a module while another updates tests against the old interface. forge-orchestrator adds
file-level locking, task planning, drift detection, and a shared knowledge base so Claude Code,
Codex CLI, and Gemini CLI can work the same codebase safely.

---

## Register 3 — Canonical paragraph (80–150 words, forge.nxtg.ai /orchestrator hero block)

Most multi-agent AI development breaks when tools don't know about each other.
forge-orchestrator is the policy core that makes it work: a single 4.7 MB Rust binary with zero
runtime dependencies that coordinates Claude Code, Codex CLI, and Gemini CLI on shared codebases.
File-level locking prevents concurrent write conflicts. A knowledge flywheel captures decisions and
patterns across tool sessions so the next agent starts with context, not from scratch. Drift
detection compares in-progress work against the original spec before divergence compounds. An
11-tool MCP server lets any connected AI client query and update orchestration state via stdio.
Local-first, no cloud account, no daemon. Runs headless in CI or as a live TUI dashboard.

*(112 words)*

---

## Register 4 — Bullet evidence (install + numbers for landing page / Show HN)

- **Install**: `curl -fsSL https://forge.nxtg.ai/install.sh | sh`
- **Binary**: 4.7 MB — LTO + stripped; musl static build available for glibc <2.39
- **Tests**: 378 (356 unit + 10 CLI + 12 MCP integration)
- **MCP tools**: 11 — `forge_get_tasks`, `forge_claim_task`, `forge_complete_task`,
  `forge_get_state`, `forge_get_plan`, `forge_capture_knowledge`, `forge_get_knowledge`,
  `forge_check_drift`, `forge_get_health`, `forge_set_project`, `forge_get_events`
- **License**: FSL-1.1-ALv2 — converts to Apache-2.0 on 2028-03-18
- **Version**: 1.5.0
- **Repo**: https://github.com/nxtg-ai/forge-orchestrator

---

## Wedge rationale

The differentiating claim is **multi-tool orchestration** — not speed, not binary size, not Rust.
Single-tool agent frameworks (LangGraph, CrewAI, AutoGen) coordinate agents inside one tool.
forge-orchestrator coordinates across tools that have no native awareness of each other.
Speed and size are supporting evidence that make the claim credible; they are not the wedge.
