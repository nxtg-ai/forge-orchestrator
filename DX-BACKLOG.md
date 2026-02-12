# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (12 remaining)

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

### DX-025: Plan --generate Overwrites Completed Tasks (CRITICAL BUG)
- **Priority:** CRITICAL
- **Where:** `src/cli/plan.rs` (task file generation)
- **Problem:** Running `plan --generate` a second time reuses T-001–T-013 IDs, overwriting previously completed tasks. History is lost. Dashboard shows stale "Completed: 4/17" from old tasks that weren't overwritten.
- **Solution (two options, pick one):**
  1. **Append mode:** Find the highest existing task ID (e.g., T-017) and start new tasks from T-018. Never reuse IDs.
  2. **Archive mode:** Move old `.forge/tasks/T-*.json` to `.forge/tasks/archive/` before generating new plan. Clean slate but history preserved.
- **Recommendation:** Option 1 (append). Simpler, no data loss, task IDs are monotonically increasing.
- **Also:** Add a `generation` or `plan_version` field to task JSON so the dashboard can group tasks by plan run.

### DX-026: Tab/Enter/Focus Keys Laggy Under Load
- **Priority:** HIGH
- **Where:** `src/cli/dashboard.rs` (event loop) + `src/tui/app.rs` (key handling)
- **Problem:** When 3 agents are streaming output, Tab and Enter/f are unresponsive. User has to press repeatedly before the key registers. Feels broken.
- **Root cause (likely):** The `tokio::select!` in the main loop processes agent events and key events in the same select. Under heavy agent output, the agent channel floods and key events get starved.
- **Solution:**
  1. **Priority key handling:** Process ALL pending key events before agent events in each tick. Use `try_recv()` in a loop to drain the key channel first.
  2. **Or separate tasks:** Move agent event processing to a separate tokio task that updates shared state, so the key handler is never blocked.
  3. **Tick rate:** Current tick might be too slow. Reduce to 50ms for snappier key response.
- **Test:** Run dashboard with 3 agents streaming heavily. Tab should register on first press every time.

### DX-027: User-Spawnable Shell Panes
- **Priority:** HIGH (stepping stone to Stargate)
- **Where:** `src/tui/app.rs` + `src/tui/ui.rs`
- **Problem:** User can't open a new terminal while agents work. They're stuck watching. Want to run `ls`, `git status`, `npm test` etc. while waiting.
- **Solution:**
  1. New key: `+` or `n` = "new pane" → spawns a shell (user's $SHELL) in a new PTY pane
  2. The pane appears in the agent grid (expanding from 2x2 to 2x3 or scrollable)
  3. When focused, keystrokes go to the shell (not to dashboard controls)
  4. `Ctrl+D` or `exit` closes the pane
- **This is DX-024 (Stargate) for user shells.** If we can spawn a user shell in a pane, we can spawn agent TUIs in panes too.
- **Prerequisite:** DX-026 (key handling must be reliable first)

### DX-028: Git Discipline — Auto-Commit Per Task
- **Priority:** HIGH
- **Where:** `src/tui/app.rs` (post-task-completion hook) + adapter configs
- **Problem:** Agents complete 17 tasks but commit NOTHING. All work sits as uncommitted changes. A stray `git checkout .` destroys everything. Zero traceability — can't `git blame` to see which agent wrote what.
- **Solution (three tiers, implement progressively):**

  **Tier 1 — Post-task auto-commit (minimum viable):**
  1. When a task completes successfully (exit 0), the dashboard runs:
     ```
     git add -A && git commit -m "feat(T-007): Implement FallbackPlanner Stub"
     ```
  2. Commit message format: `type(T-ID): Task title` where type = feat/test/docs/fix based on task_type
  3. Add `--no-gpg-sign` flag (common in CI/automated contexts)
  4. If commit fails (nothing to commit, merge conflict), log warning but don't fail the task
  5. Config: `forge config git.auto_commit true|false` (default: true)

  **Tier 2 — Git worktrees for parallel isolation:**
  1. Each agent gets its own `git worktree` branching from main:
     - `git worktree add .forge/worktrees/claude-T-001 -b forge/T-001`
     - `git worktree add .forge/worktrees/codex-T-002 -b forge/T-002`
  2. Agent's CWD is set to its worktree (not the main working tree)
  3. No merge conflicts between parallel agents — each has its own branch
  4. On task completion: commit in worktree, then merge to main:
     ```
     cd .forge/worktrees/codex-T-002
     git add -A && git commit -m "feat(T-002): Implement ControlEngine"
     cd ../../..
     git merge forge/T-002 --no-edit
     git worktree remove .forge/worktrees/codex-T-002
     git branch -d forge/T-002
     ```
  5. If merge conflicts: mark task as `needs_merge`, flag in dashboard, human resolves

  **Tier 3 — Branch strategy options (configurable):**
  - `forge config git.strategy single` — All agents commit to current branch (Tier 1)
  - `forge config git.strategy worktree` — Each agent gets a worktree (Tier 2)
  - `forge config git.strategy branch` — Each agent gets a branch, no worktree (lightweight alternative)
  - Default: `single` (simplest, works for most projects)

- **Git worktree primer:**
  - `git worktree` lets you check out multiple branches simultaneously in different directories
  - Each worktree shares the same `.git` repo but has independent working tree + index
  - Perfect for parallel agents: no conflicts, independent staging areas
  - Cleanup: `git worktree prune` removes stale worktrees
  - Limitation: Can't have two worktrees on the same branch

- **Industry patterns:**
  - Claude Code teams use worktrees for parallel teammates
  - Codex runs in a sandbox (ephemeral container per task) — no git needed inside
  - Gemini CLI has no built-in git strategy

## Completed Items (20 of 29)

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
