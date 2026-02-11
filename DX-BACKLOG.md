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
- **Where:** New `src/tui/` module + new `src/cli/dashboard.rs`
- **Crates:** `ratatui = "0.29"`, `crossterm = "0.28"` (tokio already present)

#### Vision
```
forge dashboard
┌─ FORGE DASHBOARD ── voice-jib-jab ── 3/9 tasks complete ─────────┐
│                                                                    │
│  TASK BOARD                                                        │
│  T-001 [██████████] claude  ✓ done    T-006 [████░░░░░░] codex    │
│  T-002 [██████████] codex   ✓ done    T-007 [waiting...] gemini   │
│  T-003 [██████████] claude  ✓ done    T-008 [blocked] gemini      │
│  T-004 [██████░░░░] codex   running   T-009 [blocked] gemini      │
│  T-005 [░░░░░░░░░░] codex   pending                               │
│                                                                    │
├─ agent: claude (T-004) ──────────┬─ agent: codex (T-006) ─────────┤
│                                  │                                  │
│  > Analyzing control engine...   │  > Implementing PolicyGate...    │
│  > Designing state machine       │  > Writing cancel handler        │
│  > Defining event schemas        │  > Adding escalation flow        │
│  >                               │  > Running tests...              │
│                                  │                                  │
├─ agent: gemini (idle) ───────────┤─ logs ──────────────────────────┤
│                                  │                                  │
│  Waiting for T-004, T-006        │  14:32:01 T-001 completed (45s) │
│                                  │  14:32:02 T-002 started          │
│                                  │  14:33:15 T-002 completed (73s) │
│                                  │  14:33:16 T-004, T-006 started   │
│                                  │                                  │
└──────────────────────────────────┴──────────────────────────────────┘
  [q] quit  [↑↓] select task  [enter] view detail  [p] pause  [r] retry failed
```

#### Implementation — 3 phases (do all three)

**Phase 1: Static dashboard (`forge dashboard` reads task board, renders with ratatui)**

New files:
- `src/cli/dashboard.rs` — CLI entry point, calls `tui::run_dashboard()`
- `src/tui/mod.rs` — module root
- `src/tui/app.rs` — App state struct (tasks, selected index, logs, agent outputs)
- `src/tui/ui.rs` — ratatui rendering (layout, task table widget, agent pane widgets, log widget)
- `src/tui/event.rs` — crossterm event handling (keyboard input, tick timer)

Dependencies to add in Cargo.toml:
```toml
ratatui = "0.29"
crossterm = "0.28"
```

App state:
```rust
pub struct App {
    pub tasks: Vec<Task>,           // loaded from .forge/tasks/
    pub agent_outputs: HashMap<String, Vec<String>>,  // agent_name -> output lines
    pub logs: Vec<String>,          // timestamped event log
    pub selected_task: usize,       // cursor position in task table
    pub running: bool,              // false = quit
    pub tick_rate: Duration,        // 250ms
}
```

Layout (ratatui):
```rust
// Top: task board table (30% height)
// Middle: 2x2 grid of agent panes (50% height)
// Bottom: event log (20% height)
// Use Layout::vertical() + Layout::horizontal() for the grid
```

Keyboard:
- `q` / `Ctrl+C` → quit
- `↑↓` → select task in table
- `Enter` → show task detail (description, acceptance criteria)
- `r` → retry selected failed task

Main loop:
```rust
pub async fn run_dashboard(project_root: &Path) -> anyhow::Result<()> {
    // 1. Enter raw mode (crossterm)
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 2. Load initial state
    let app = App::new(project_root)?;

    // 3. Event loop
    loop {
        terminal.draw(|f| ui::render(f, &app))?;
        if crossterm::event::poll(app.tick_rate)? {
            if let Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    // ... handle other keys
                }
            }
        }
        // Refresh task states from disk every tick
        app.refresh_tasks()?;
    }

    // 4. Restore terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
```

Register in `main.rs`:
```rust
("dashboard", _) => dashboard::execute(&project_root)?,
```

Add to clap subcommands:
```rust
Command::new("dashboard")
    .about("Open the live TUI dashboard")
```

**Phase 2: Live execution (spawn agents, pipe stdout to panes)**

When the dashboard starts, it also starts executing tasks (or add `--watch` for read-only mode):

```rust
// In the async event loop, spawn agent processes for unblocked tasks:
async fn spawn_task(task: &Task, project_root: &Path, tx: mpsc::Sender<AgentEvent>) {
    let adapter = get_adapter(&task.assigned_to);
    let mut cmd = adapter.build_command(task, project_root, &auth_mode, &permissions);

    // Pipe stdout/stderr
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = tokio::process::Command::from(cmd).spawn()?;

    // Stream stdout line-by-line to the TUI via channel
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        tx.send(AgentEvent::Output {
            task_id: task.id.clone(),
            line,
        }).await?;
    }

    let status = child.wait().await?;
    tx.send(AgentEvent::Completed {
        task_id: task.id.clone(),
        success: status.success(),
    }).await?;
}
```

AgentEvent enum:
```rust
enum AgentEvent {
    Output { task_id: String, line: String },
    Completed { task_id: String, success: bool },
    Error { task_id: String, message: String },
}
```

The main loop uses `tokio::select!` to handle both terminal events and agent events:
```rust
loop {
    tokio::select! {
        // Terminal input
        _ = tick_interval.tick() => {
            terminal.draw(|f| ui::render(f, &app))?;
        }
        // Agent output
        Some(event) = agent_rx.recv() => {
            match event {
                AgentEvent::Output { task_id, line } => {
                    app.agent_outputs.entry(task_id).or_default().push(line);
                }
                AgentEvent::Completed { task_id, success } => {
                    app.mark_completed(&task_id, success)?;
                    app.schedule_unblocked_tasks(&tx)?; // auto-advance
                }
            }
        }
    }
}
```

**Phase 3: Dependency-aware auto-scheduling**

When a task completes:
1. Update its status in `.forge/tasks/`
2. Check all pending tasks — any whose `depends_on` are now all completed become unblocked
3. Spawn unblocked tasks (up to `--parallel N` concurrent, default 3)
4. Continue until all tasks done or all remaining are blocked/failed

```rust
impl App {
    fn schedule_unblocked_tasks(&mut self, tx: &mpsc::Sender<AgentEvent>) {
        let completed_ids: HashSet<String> = self.tasks.iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();

        let active_count = self.tasks.iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count();

        for task in &mut self.tasks {
            if task.status != TaskStatus::Pending { continue; }
            if active_count >= self.max_parallel { break; }

            let deps_met = task.depends_on.iter()
                .all(|dep| completed_ids.contains(dep));

            if deps_met {
                task.status = TaskStatus::InProgress;
                let tx = tx.clone();
                let task_clone = task.clone();
                tokio::spawn(async move {
                    spawn_task(&task_clone, &project_root, tx).await;
                });
            }
        }
    }
}
```

#### CLI flags
```
forge dashboard              # Start dashboard + auto-execute tasks
forge dashboard --watch      # Read-only mode (just show task board, no execution)
forge dashboard --parallel 4 # Max 4 concurrent agents (default: 3)
```

#### Test strategy
- Unit test `App::schedule_unblocked_tasks()` with mock task graphs
- Unit test `ui::render()` using ratatui's `TestBackend`
- Integration test: start dashboard with a simple 3-task plan, verify all complete
- All existing 59 tests must still pass

#### Key constraints
- Terminal must be fully restored on panic (use `std::panic::set_hook` to disable raw mode)
- Agent output lines are capped at 200 per pane (ring buffer) to prevent memory growth
- Tick rate: 250ms (4 fps — smooth enough, light on CPU)
- The dashboard must work over SSH (no mouse required, keyboard-only navigation)

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
