# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).
> DX-001 through DX-008, DX-012, DX-013, DX-015, DX-016 already fixed and committed.

## Open Items (Priority Order)

### DX-009: Spinner / Progress Indicator During LLM Calls
- **Priority:** HIGH — DO THIS BEFORE DX-017 (codebase scan adds more latency)
- **Where:** `src/cli/plan.rs` (plan generation), `src/cli/run.rs` (task execution)
- **Problem:** Silent wait while OpenAI/Claude thinks (~3-30 seconds). No feedback. DX-017 will make this worse by adding a codebase scan phase.
- **Solution:** Use `indicatif` crate for terminal spinners.
- **Implementation:**

#### Step 1: Add dependency
```toml
# Cargo.toml
indicatif = "0.17"
```

#### Step 2: Plan generation spinners in `src/cli/plan.rs`
Replace the current static `println!` progress with phased spinners:
```rust
use indicatif::{ProgressBar, ProgressStyle};

let spinner_style = ProgressStyle::default_spinner()
    .template("{spinner:.cyan} {msg}")
    .unwrap()
    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);

// Phase 1: Loading spec
let sp = ProgressBar::new_spinner();
sp.set_style(spinner_style.clone());
sp.set_message("Loading spec...");
sp.enable_steady_tick(std::time::Duration::from_millis(80));
// ... load spec ...
sp.finish_with_message("✓ Spec loaded (607 lines)");

// Phase 2: Scanning codebase (placeholder for DX-017)
// sp.set_message("Scanning codebase...");

// Phase 3: Decomposing into tasks (THE SLOW PART — 5-30 seconds)
let sp = ProgressBar::new_spinner();
sp.set_style(spinner_style.clone());
sp.set_message("Decomposing spec into tasks...");
sp.enable_steady_tick(std::time::Duration::from_millis(80));
let tasks = brain.decompose_plan(&spec_content, &tools)?;
sp.finish_with_message(format!("✓ Generated {} tasks", tasks.len()));

// Phase 4: Assigning agents
let sp = ProgressBar::new_spinner();
sp.set_style(spinner_style.clone());
sp.set_message("Assigning agents to tasks...");
sp.enable_steady_tick(std::time::Duration::from_millis(80));
for task in &mut tasks { ... }
sp.finish_with_message("✓ Agents assigned");

// Phase 5: Writing to disk
let sp = ProgressBar::new_spinner();
sp.set_style(spinner_style.clone());
sp.set_message("Writing task board...");
sp.enable_steady_tick(std::time::Duration::from_millis(80));
// ... write tasks + plan.md ...
sp.finish_with_message("✓ Plan written to .forge/plan.md");
```

#### Step 3: Task execution spinner in `src/cli/run.rs`
The headless `forge run --task T-001 --agent claude` is also silent during execution:
```rust
let sp = ProgressBar::new_spinner();
sp.set_style(spinner_style);
sp.set_message(format!("Running {} on {}...", agent_name, task.id));
sp.enable_steady_tick(std::time::Duration::from_millis(80));
let result = adapter.execute_headless(&task, project_root, &auth_mode, &permissions)?;
sp.finish_with_message(match result.success {
    true => format!("✓ {} completed", task.id),
    false => format!("✗ {} failed", task.id),
});
```

#### Step 4: Suppress `eprintln!` debug noise
The `[forge-brain]` debug prints in `openai.rs` clash with spinners (they write to stderr while the spinner is on stdout). Either:
- Remove them (the spinner replaces their purpose)
- Or gate them behind a `--verbose` flag

#### Expected UX after fix
```
forge plan --generate
  ⠹ Loading spec...
  ✓ Spec loaded (607 lines)
  ⠼ Decomposing spec into tasks...     ← this spins for 5-15 seconds
  ✓ Generated 17 tasks
  ⠧ Assigning agents to tasks...
  ✓ Agents assigned
  ⠏ Writing task board...
  ✓ Plan written to .forge/plan.md

  [task table renders here]
```

- **Stretch:** Skeleton rendering — show table frame first, fill rows as they stream in from LLM

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

### DX-017: Codebase-Aware Plan Generation (Spec vs Reality Diff)
- **Priority:** CRITICAL — without this, forge generates tasks for already-built features
- **Where:** `src/cli/plan.rs` (new `scan_codebase()` function), `src/brain/openai.rs` (updated system prompt)
- **Problem:** `plan --generate` only reads the spec file and generates tasks for everything it describes. It has ZERO awareness of existing source code. If the project is already 80% built, it still generates tasks for 100% of the spec.
- **Discovery:** Dogfood on voice-jib-jab — project already fully implemented, but forge generated 17 tasks to "design" and "implement" components that already exist.

#### Root Cause
The data flow is:
```
resolve_plan_input() → reads spec-jib-jab.md (607 lines)
  → sends to brain.decompose_plan(spec, tools)
    → brain sees ONLY the spec, not the codebase
      → generates tasks for everything in spec
```

#### Solution: Add codebase scan to plan generation

**Step 1: New function `scan_codebase()` in `src/cli/plan.rs`**

Scan the project and build a concise inventory:
```rust
fn scan_codebase(project_root: &Path) -> anyhow::Result<String> {
    let mut inventory = String::new();

    // 1. Source file tree (just paths, no content)
    //    Walk src/, lib/, app/ directories
    //    List all .ts, .rs, .py, .go, .js, .tsx, .jsx files
    //    Format: "src/event-bus.ts (142 lines)"

    // 2. Key exports/structures (first 5 lines of each file, or grep for
    //    export/pub/class/struct/interface/function patterns)
    //    Format: "src/event-bus.ts: export class EventBus, export interface Event"

    // 3. Test files inventory
    //    List all *test*, *spec* files
    //    Format: "tests/event-bus.test.ts (89 lines)"

    // 4. Package dependencies (from package.json / Cargo.toml)
    //    Already gathered in gather_project_context() — reuse

    Ok(inventory)
}
```

**Keep it lightweight** — file names + line counts + export signatures. Do NOT read full file contents (would blow up the token budget). Target: ~100-200 lines of inventory for a medium project.

**Step 2: Update `generate_plan()` to call scan**

In `generate_plan()`, after loading the spec, scan the codebase:
```rust
// After spec_content is loaded:
let codebase_inventory = scan_codebase(project_root)?;
println!("  {} Codebase scanned ({} source files)", "✓".green(), file_count);

// Combine for the brain
let brain_input = format!(
    "PROJECT SPECIFICATION:\n\n{spec_content}\n\n---\n\n\
     EXISTING CODEBASE INVENTORY:\n\n{codebase_inventory}\n\n---\n\n\
     Generate tasks ONLY for what is missing, incomplete, or needs updating.\n\
     Do NOT generate tasks for features that already exist in the codebase."
);
let tasks = brain.decompose_plan(&brain_input, &tools_for_brain)?;
```

**Step 3: Update OpenAI brain system prompt** in `src/brain/openai.rs`

Add to the system prompt in `decompose_plan()`:
```
The input may include both a spec AND an existing codebase inventory.
If a codebase inventory is provided:
- Do NOT create tasks for features/modules that already exist
- Focus on gaps: what's in the spec but NOT in the codebase
- Create "review" tasks for existing code that may need updates
- Create "test" tasks for existing code that lacks tests
- If the codebase already covers the full spec, output fewer or zero tasks
```

**Step 4: Update rule-based brain** in `src/brain/rule_based.rs`

The rule-based brain uses heading decomposition. Add a filter:
- If the brain input contains `EXISTING CODEBASE INVENTORY`, extract file names
- For each heading-based task, check if a matching file already exists
- Skip tasks where the file already exists (or mark as "review" instead of "implement")

#### File scanning patterns by language

| Language | Source dirs | Extensions | Export patterns |
|----------|-----------|------------|-----------------|
| TypeScript/JS | `src/`, `lib/`, `app/` | `.ts`, `.tsx`, `.js`, `.jsx` | `export (class\|function\|const\|interface\|type)` |
| Rust | `src/` | `.rs` | `pub (fn\|struct\|enum\|trait\|mod)` |
| Python | `src/`, project name dir | `.py` | `class `, `def `, `__all__` |
| Go | `.`, `cmd/`, `internal/`, `pkg/` | `.go` | `func `, `type ` |

#### Constraints
- **Token budget:** Keep inventory under 4000 tokens (~200 lines). Large projects need truncation
- **Depth limit:** Only scan 3 levels deep. Don't recurse into node_modules, target, .git, etc.
- **Ignore patterns:** `.git`, `node_modules`, `target`, `dist`, `build`, `__pycache__`, `.forge`
- **Performance:** Use `walkdir` crate for efficient traversal (already a transitive dep via `glob`)

#### Expected behavior after fix
```
forge plan --generate
  → Reading spec: spec-jib-jab.md (607 lines)
  → Scanning codebase... (34 source files found)
  → 28 of 34 files match spec components
  → Decomposing GAPS into tasks...
  ✓ Generated 3 tasks (instead of 17)

  ID       Title                          Agent      Type         Status
  -----------------------------------------------------------------------
  T-001    Add missing unit tests for...  gemini     test         pending
  T-002    Review session manager for...  claude     review       pending
  T-003    Document API endpoints         gemini     document     pending
```

#### Test cases
1. **Greenfield project** (no src/): Should generate full task set (same as today)
2. **Fully built project** (all features exist): Should generate 0-3 tasks (review/test/doc only)
3. **Partially built** (some files exist): Should generate tasks only for missing pieces
4. **Spec with no codebase section**: Should work identically to today (backward compat)



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

## Completed Items

- DX-001 through DX-008: All fixed (commit `f99193b`)
- DX-012: Auth config (commit `6945863`)
- DX-013: Async execution via tokio (commit `8740251`)
- DX-015: Yolo permissions mode (commit `6945863`)
- DX-016: Smart Claude adapter — task-type-aware `--allowedTools` scoping (v0.2.1)

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Strip API keys (default)
forge config claude.auth api             # Pass API keys through
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
