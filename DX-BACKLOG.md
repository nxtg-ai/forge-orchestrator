# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (9 remaining)

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

### DX-018: Rate Limit Backoff for Agent Processes
- **Priority:** High
- **Where:** `src/tui/app.rs` (agent spawning + retry logic)
- **Problem:** When an agent (especially Gemini) hits API rate limits, the process retries aggressively and can get the user banned. Dashboard shows "Trying to reach gemini-2.5-flash (Attempt 2/3)" but has no backoff strategy.
- **Solution:** Exponential backoff with jitter when agent process exits with rate-limit indicators:
  1. Detect rate limit signals in agent stdout/stderr (429, "rate limit", "quota exceeded", "Attempt N/M")
  2. Wait before re-scheduling: 10s → 30s → 60s → 120s (exponential with jitter)
  3. Show backoff timer in agent pane: "Rate limited. Retrying in 45s..."
  4. Cap at 5 retries, then mark task as `failed` with "rate_limited" reason
  5. Never retry more than 1 agent at a time to the same API (stagger retries)
- **Existing patterns:** `start.rs` already has transient error detection (credit balance, rate limits, timeouts, 500 errors). Reuse those patterns.

### DX-019: Gemini Adapter Headless Fix (SHIPPED — needs commit)
- **Priority:** CRITICAL (already fixed in code, needs commit)
- **Where:** `src/adapters/gemini.rs`
- **Problem:** Gemini adapter launched in interactive mode (no `-p` flag), no auto-approval (`--yolo`), causing approval prompts and tool restrictions in headless execution.
- **Fix:** Added `-p` flag for headless mode, `--yolo` + `--sandbox=false` in yolo permissions mode.
- **Status:** Code patched, binary installed. Needs commit + push.

### DX-021: Dashboard Should Clean Up Task Status on Quit/Restart
- **Priority:** CRITICAL
- **Where:** `src/tui/app.rs` + `src/cli/dashboard.rs`
- **Problem:** When dashboard quits (q/Esc/Ctrl+C), tasks that were "in_progress" stay that way on disk. On restart, the scheduler sees them as already running and won't re-spawn. Result: orphaned tasks that never complete.
- **Solution (two parts):**
  1. **On quit:** Before exiting, reset all tasks the dashboard was actively running back to `"pending"` status. The dashboard knows which PIDs it spawned — if it kills them, it should also reset their task files.
  2. **On startup:** Scan all tasks for `"in_progress"` status. Check if there's actually a running process for them (there won't be on fresh launch). Reset any orphaned in-progress tasks to `"pending"`.
  3. **Bonus:** `r` key should also work on "in_progress" tasks (not just "failed"), resetting and re-spawning them.
- **Root cause of this session's bug:** Dashboard quit with T-007 in_progress → restarted → T-007 stuck forever.

### DX-020: Key Legend in Dashboard Footer
- **Priority:** Low
- **Where:** `src/tui/ui.rs`
- **Problem:** No key hints visible — user has to guess keyboard shortcuts.
- **Solution:** Add a single-line footer bar below the event log showing: `q:Quit | Tab:Focus | ↑↓:Navigate | Enter:Detail | r:Retry | ?:Help`
- **Pattern:** Same as vim/htop/nano status bars. Use `ratatui::widgets::Paragraph` with `Style::new().fg(Color::DarkGray)`.

### DX-022: Dashboard Should NOT Auto-Exit on Completion
- **Priority:** CRITICAL
- **Where:** `src/tui/app.rs` (scheduler loop / completion detection)
- **Problem:** When the last task completes, the dashboard immediately exits to terminal. This is shocking — user expects a summary, celebration, or at minimum a "press q to quit" state.
- **Solution (progressive):**
  1. **Minimum:** When all tasks complete, show a completion banner ("All 17/17 tasks done!") and STAY OPEN. User presses `q` to exit.
  2. **Better:** Show a summary panel: tasks completed, time elapsed, agents used, failures, knowledge captured.
  3. **Best:** Keep agent panes visible and scrollable so user can review terminal output at their leisure.
- **User quote:** "once the last item was completed.. the dashboard just *SHUT DOWN* .. it was like a shock"

### DX-023: Interactive Terminal Panes (Scrollable, Tab-Switchable)
- **Priority:** High
- **Where:** `src/tui/ui.rs` + `src/tui/app.rs`
- **Problem:** Agent panes are read-only, not scrollable, and can't be individually focused/expanded.
- **Solution:**
  1. Terminal panes should be scrollable (↑↓ when focused)
  2. Tab key should cycle focus between panes (visual border highlight on focused pane)
  3. Enter on a focused pane could expand it to full-screen (press Esc to return to grid)
  4. Each pane's ring buffer (200 lines) should be navigable
- **Future (DX-024):** If panes were actual PTY bridges, user could TYPE into them — turning the dashboard into a true multi-terminal multiplexer.

### DX-024: Forge Stargate — Embedded Interactive Agent TUIs
- **Priority:** VISION (future)
- **Where:** New architecture — PTY bridge per agent pane
- **Problem:** Current agent panes show captured stdout text. The dream is embedding ACTUAL running TUIs (Claude Code's TUI, Gemini's TUI, Codex's TUI) inside forge dashboard panes.
- **Solution:** Instead of spawning headless `-p` processes, spawn interactive CLIs in PTY sessions. Each pane becomes a real terminal with input/output. User can tab between them and interact directly.
- **Why "Stargate":** It's a portal into each AI's universe. Three portals, one command center.
- **Technical:** `portable-pty` or `pty-process` crate for PTY allocation, pipe each PTY's output to a ratatui pane, forward keystrokes when pane is focused.
- **Prerequisite:** DX-023 (scrollable/focusable panes) must ship first.

## Completed Items (16 of 20)

| DX | Description | Version |
|----|-------------|---------|
| DX-019 | Gemini adapter headless fix (`-p` + `--yolo`) | v0.3.1 |

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
