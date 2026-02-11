# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (3 remaining)

### DX-010: Status Should Show Full Task Table with Dependencies
- **Priority:** High
- **Where:** `src/cli/status.rs`
- **Problem:** Status shows only summary (Total: 17, Pending: 17, progress bar)
- **Solution:** Render the same table as `plan --generate` output but with live statuses
- **Critical:** Show which tasks are blocked and which can run in parallel
- **Data needed:** Task dependencies (blocked_by field already exists in Task struct)

### DX-011: Autonomous Loop Mode (`forge run` with no args)
- **Priority:** CRITICAL
- **Where:** `src/cli/run.rs` (new mode) or new `src/cli/orchestrate.rs`
- **Problem:** `run --task T-001 --agent claude` runs one task then exits
- **Solution:** `forge run` (no args) = autonomous loop:
  1. Read task board
  2. Find all unblocked pending tasks
  3. Run them in parallel (respecting dependencies)
  4. Update statuses in real-time
  5. Keep going until all done or failure blocks progress
- **Flags:** `--dry-run` (show what WOULD run), `--parallel N` (max concurrent)
- **This is CEO mode — press one button, walk away, come back to results**
- **Note:** `forge start --ceo` already does multi-pass retry. DX-011 is about making `forge run` (no args) do the same with dependency-aware scheduling.

### DX-014: Agent Pane TUI — THE KILLER FEATURE
- **Priority:** THIS IS THE PRODUCT
- **Where:** New `src/tui/` module
- **Crates:** `ratatui`, `crossterm`, `tokio`
- **Vision:**
```
forge dashboard
  ┌─────────────────────────────────────────────────────┐
  │  TASK BOARD                                          │
  │  T-001 [██████░░░░] claude  T-003 [████░░░░░░] claude│
  │  T-002 [░░░░░░░░░░] codex   T-016 [waiting...] gemini│
  ├──────────────────────┬──────────────────────────────-─┤
  │  agent: claude (T-001)│  agent: codex (T-002)         │
  │  > Reading spec...   │  > Implementing event bus...   │
  │  > Designing schema  │  > Writing EventBus struct     │
  ├──────────────────────┼────────────────────────────────┤
  │  agent: claude (T-003)│  agent: gemini (T-016)        │
  │  > [blocked]         │  > [waiting for T-001]         │
  └──────────────────────┴────────────────────────────────┘
```
- Each agent gets its own pane with live streaming output
- Tasks auto-advance as dependencies complete
- Parallel execution respecting the dependency graph
- **Technical path:** `.spawn()` + `tokio` async + ratatui render loop + crossterm raw mode

## Architecture Notes

### Target flow (for DX-011/014):
```
orchestrate.rs → spawn tasks based on dependency graph
  → adapter.execute_async() → Command::new("claude").spawn() → non-blocking
  → tokio::select! on multiple child processes
  → stream stdout/stderr to TUI panes via channels
  → on completion: update task status, unblock dependents, schedule next
```

### Dependency order:
```
DX-010 (status table) — standalone
DX-011 (auto loop) → DX-014 (TUI dashboard)
```

DX-013 (async spawn) is already done — the foundation is in place.

## Completed Items (14 of 17)

| DX | Description | Version |
|----|-------------|---------|
| DX-001–008 | Init, plan, config, status fixes | v0.2.0 |
| DX-009 | Spinner / progress indicators (indicatif) | v0.2.2 |
| DX-012 | Per-agent auth config (subscription/api) | v0.2.0 |
| DX-013 | Async execution via tokio | v0.2.0 |
| DX-015 | Yolo permissions mode | v0.2.0 |
| DX-016 | Smart Claude adapter (task-type-aware tools) | v0.2.1 |
| DX-017 | Codebase-aware plan generation (spec vs reality) | v0.2.2 |

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Strip API keys (default)
forge config claude.auth api             # Pass API keys through
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
