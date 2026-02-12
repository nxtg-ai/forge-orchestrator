# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (2 remaining — vision/future)

### DX-024: Forge Stargate — Embedded Interactive Agent TUIs
- **Priority:** VISION (v2.0)
- **Where:** New architecture — PTY bridge per agent pane
- **Problem:** Current agent panes show captured stdout text. The dream is embedding ACTUAL running TUIs (Claude Code's TUI, Gemini's TUI, Codex's TUI) inside forge dashboard panes.
- **Solution:** Instead of spawning headless `-p` processes, spawn interactive CLIs in PTY sessions. Each pane becomes a real terminal with input/output. User can tab between them and interact directly.
- **Why "Stargate":** It's a portal into each AI's universe. Three portals, one command center.
- **Technical:** `portable-pty` or `pty-process` crate for PTY allocation, pipe each PTY's output to a ratatui pane, forward keystrokes when pane is focused.
- **Prerequisite:** DX-023 (done) and DX-027 (done) — scrollable/focusable panes + shell panes prove the pattern.

### DX-028 Tier 2/3: Git Worktrees for Parallel Isolation
- **Priority:** FUTURE (v1.x)
- **Where:** `src/tui/app.rs` + new `src/core/git.rs`
- **Problem:** Tier 1 auto-commit (done in v1.0) commits to current branch. When 3 agents edit the same files in parallel, conflicts are possible.
- **Solution:**
  - **Tier 2:** Each agent gets a `git worktree`, commits independently, merges on completion
  - **Tier 3:** `forge config git.strategy single|worktree|branch` — user picks strategy
- **See full spec in git history (DX-028 original description)**

## Completed Items (29 of 31)

| DX | Description | Version |
|----|-------------|---------|
| DX-001–008 | Init, plan, config, status fixes | v0.2.0 |
| DX-009 | Spinner / progress indicators (indicatif) | v0.2.2 |
| DX-010 | Full task table with dependencies in `forge status` | v1.0.0 |
| DX-011 | Headless autonomous mode (`forge run` no args) | v1.0.0 |
| DX-012 | Per-agent auth config (subscription/api) | v0.2.0 |
| DX-013 | Async execution via tokio | v0.2.0 |
| DX-014 | TUI dashboard with live agent panes | v0.3.0 |
| DX-015 | Yolo permissions mode | v0.2.0 |
| DX-016 | Smart Claude adapter (task-type-aware tools) | v0.2.1 |
| DX-017 | Codebase-aware plan generation (spec vs reality) | v0.2.2 |
| DX-018 | Rate limit backoff (exponential with jitter) | v1.0.0 |
| DX-019 | Gemini adapter headless fix (`-p` + `--yolo`) | v1.0.0 |
| DX-020 | Key legend in dashboard footer | v1.0.0 |
| DX-021 | Orphan task cleanup on quit/restart | v1.0.0 |
| DX-022 | Dashboard stays open on completion | v1.0.0 |
| DX-023 | Interactive terminal panes (scrollable, focusable) | v1.0.0 |
| DX-025 | Monotonic task IDs (never overwrite completed) | v1.0.0 |
| DX-026 | Priority key handling (no lag under load) | v1.0.0 |
| DX-027 | User-spawnable shell panes | v1.0.0 |
| DX-028 | Git auto-commit per task (Tier 1) | v1.0.0 |

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Strip API keys (default)
forge config claude.auth api             # Pass API keys through
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
