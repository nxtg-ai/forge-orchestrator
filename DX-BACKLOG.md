# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).
> DX-001 through DX-008, DX-012, DX-015 already fixed and committed.

## Open Items (Priority Order)

### DX-009: Spinner / Progress Indicator During LLM Calls
- **Priority:** Medium
- **Where:** `src/cli/plan.rs` (plan generation), `src/adapters/*.rs` (task execution)
- **Problem:** Silent wait while OpenAI/Claude thinks (~3-30 seconds). No feedback.
- **Solution:** Use `indicatif` crate for terminal spinners. Show phased progress:
  - `Analyzing spec...` → `Decomposing into tasks...` → `Assigning agents...`
  - Even if LLM does it in one shot, the phases give perceived speed
- **Stretch:** Skeleton rendering — show table frame, fill rows as they arrive

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

### DX-013: Non-Blocking Agent Execution
- **Priority:** HIGH (prerequisite for DX-011 and DX-014)
- **Where:** `src/adapters/*.rs`
- **Problem:** `Command::new("claude").output()` blocks the thread
- **Solution:** Switch to `Command::new("claude").spawn()` + async I/O
- **Dependencies:** Add `tokio` for async runtime
- **This unlocks:** spinner (DX-009), parallel execution (DX-011), TUI panes (DX-014)

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

### Current adapter execution flow:
```
run.rs → adapter.execute_headless() → Command::new("claude").output() → blocks
```

### Target flow (for DX-011/013/014):
```
orchestrate.rs → spawn tasks based on dependency graph
  → adapter.execute_async() → Command::new("claude").spawn() → non-blocking
  → tokio::select! on multiple child processes
  → stream stdout/stderr to TUI panes via channels
  → on completion: update task status, unblock dependents, schedule next
```

### Dependency order for implementation:
```
DX-013 (async spawn) → DX-009 (spinner) → DX-011 (loop) → DX-014 (TUI)
```

DX-013 is the foundation. Everything else builds on non-blocking execution.

### DX-016: Smart Claude Adapter — Enhance, Don't Strip (CRITICAL)
- **Priority:** CRITICAL — this is what makes forge-orca the BEST way to use Claude Code
- **Where:** `src/adapters/claude.rs`
- **Problem:** Current adapter uses `claude -p` for everything — a one-shot, no-tools, no-plan invocation. This WASTES Claude Code's most powerful features: Plan mode, Agent Teams, MCP tools, CLAUDE.md context, interactive review, resume.
- **Principle:** "The DX must elevate and enhance existing capabilities of Claude, not strip them out." — Asif
- **Solution:** The Claude adapter should choose invocation strategy based on task metadata:

| Task Type | Detection | Claude Invocation |
|-----------|-----------|-------------------|
| Simple (fix/tweak) | Short description, single file | `claude -p` (current) |
| Design (architecture) | Title contains "Design", "Architect", "Plan" | `claude -p` with plan-encouraging prompt |
| Multi-file (feature) | `locked_files.len() > 2` or "Implement" keyword | Spawn with agent teams hint in prompt |
| Review (analysis) | "Review", "Test", "Document" keywords | `claude -p` with read-only `--allowedTools Read,Glob,Grep` |
| Complex (full feature) | Long description, many acceptance criteria | `claude -p --allowedTools Write,Edit,Read,Glob,Grep,Bash` with structured prompt |

- **Task metadata already exists:** The brain (gpt-4.1) classifies tasks during `plan --generate`. We can add a `task_type` field (design/implement/review/test/document) to the Task struct.
- **Claude CLI flags to leverage:**
  - `--allowedTools Write,Edit,Read,Glob,Grep,Bash` — scoped permissions (better than yolo)
  - `--model` — could use different models for different task types (haiku for simple, opus for design)
  - `--append-system-prompt` — inject task-specific context
  - `--max-turns` — limit turns for simple tasks, unlimited for complex
- **Implementation steps:**
  1. Add `task_type: Option<String>` to Task struct (design, implement, review, test, document)
  2. Have the brain classify task types during plan generation
  3. Update `ClaudeAdapter::build_command()` to read task type and choose flags
  4. Similar approach for Codex and Gemini adapters

## Completed Items

- DX-001 through DX-008: All fixed (commit `f99193b`)
- DX-012: Auth config (commit `6945863`)
- DX-013: Async execution via tokio (commit `8740251`)
- DX-015: Yolo permissions mode (commit `6945863`)

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Strip API keys (default)
forge config claude.auth api             # Pass API keys through
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
