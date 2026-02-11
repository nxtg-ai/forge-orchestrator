# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (2 remaining)

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
- **Note:** `forge start --ceo` already does multi-pass retry. DX-011 is about making `forge run` (no args) do the same with dependency-aware scheduling. Now that `forge dashboard` exists (DX-014), DX-011 may be redundant — the dashboard already does dependency-aware auto-scheduling with a TUI. Consider whether `forge run` (no args) should just launch the dashboard instead.

## Completed Items (15 of 17)

| DX | Description | Version |
|----|-------------|---------|
| DX-001–008 | Init, plan, config, status fixes | v0.2.0 |
| DX-009 | Spinner / progress indicators (indicatif) | v0.2.2 |
| DX-012 | Per-agent auth config (subscription/api) | v0.2.0 |
| DX-013 | Async execution via tokio | v0.2.0 |
| DX-014 | TUI dashboard with live agent panes | v0.3.0 |
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
