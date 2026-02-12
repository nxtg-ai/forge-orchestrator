# Assignment V1.1-2: Dashboard Phase 2 Auto-Transition + Test/Fix Loop

> Priority: HIGH | Estimated: 45-60min | Version target: v1.1.0
> **Depends on:** Assignment V1.1-1 (task model extensions + forge verify)

## The Problem

After all build tasks complete, the dashboard should automatically transition to Phase 2 (VERIFY). It should generate verify subtasks, execute them, and if a verify task fails, auto-generate a fix subtask and retry (up to 3 times). This is the "test/fix loop" — the dashboard keeps going until all tests pass or retries are exhausted.

## Part 1: Phase tracking in App struct (`src/tui/app.rs`)

Add a phase field to the `App` struct:

```rust
use crate::core::task::TaskPhase;

// In App struct:
/// Current lifecycle phase of the dashboard.
pub phase: DashboardPhase,
```

Add the enum (in app.rs or a nearby module):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPhase {
    /// Phase 1: Building — agents execute build tasks
    Build,
    /// Phase 2: Verifying — running verify subtasks, auto-fixing failures
    Verify,
    /// All phases complete — waiting for user to quit
    Complete,
}

impl std::fmt::Display for DashboardPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DashboardPhase::Build => write!(f, "BUILD"),
            DashboardPhase::Verify => write!(f, "VERIFY"),
            DashboardPhase::Complete => write!(f, "COMPLETE"),
        }
    }
}
```

Initialize as `DashboardPhase::Build` in `App::new()`.

## Part 2: Auto-transition from Build to Verify

In `App::handle_event()`, after a task completes successfully, check if all build tasks are done. If so, transition to Phase 2:

```rust
// After the existing AgentEvent::Completed success handling:
// Check for phase transition
self.check_phase_transition();
```

Add the method:

```rust
fn check_phase_transition(&mut self) {
    if self.phase != DashboardPhase::Build {
        return;
    }

    // Check if all build-phase tasks are complete
    let all_build_done = self.tasks.iter()
        .filter(|t| {
            t.phase.is_none() || t.phase == Some(TaskPhase::Build)
        })
        .all(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Failed));

    if !all_build_done {
        return;
    }

    // Transition to Verify phase
    self.phase = DashboardPhase::Verify;
    self.push_event("Phase 1 (BUILD) complete — transitioning to Phase 2 (VERIFY)");

    // Generate verify subtasks
    let task_mgr = TaskManager::new(&self.forge_dir);
    match task_mgr.generate_verify_subtasks() {
        Ok(generated) => {
            if generated.is_empty() {
                self.push_event("No verify subtasks needed — all complete");
                self.phase = DashboardPhase::Complete;
            } else {
                self.push_event(&format!("{} verify subtasks generated", generated.len()));
                // Reload tasks to pick up the new verify subtasks
                self.reload_tasks().ok();
            }
        }
        Err(e) => {
            self.push_event(&format!("Error generating verify tasks: {e}"));
        }
    }
}
```

## Part 3: Verify-phase completion detection

Update the existing `all_complete` check to be phase-aware. The dashboard currently checks if all tasks are done — now it should also check verify tasks:

```rust
// In the tick/scheduling loop, update the all_complete check:
fn check_all_complete(&mut self) {
    let all_done = self.tasks.iter().all(|t| {
        matches!(t.status, TaskStatus::Completed | TaskStatus::Failed)
    });

    if all_done && self.running_task_ids.is_empty() {
        if self.phase == DashboardPhase::Build {
            // Don't mark complete — transition to verify first
            self.check_phase_transition();
        } else if self.phase == DashboardPhase::Verify {
            self.phase = DashboardPhase::Complete;
            if !self.all_complete {
                self.all_complete = true;
                self.completed_at = Some(Instant::now());
                let verified = self.tasks.iter()
                    .filter(|t| t.phase == Some(TaskPhase::Verify) && t.status == TaskStatus::Completed)
                    .count();
                let failed = self.tasks.iter()
                    .filter(|t| t.phase == Some(TaskPhase::Verify) && t.status == TaskStatus::Failed)
                    .count();
                self.push_event(&format!(
                    "Phase 2 (VERIFY) complete — {verified} passed, {failed} failed"
                ));
            }
        }
    }
}
```

## Part 4: Test/Fix loop — auto-generate fix subtasks on verify failure

When a verify subtask fails, automatically generate a fix subtask (up to 3 retries):

In `handle_event()`, after a task fails, check if it's a verify task:

```rust
// In the failure branch of AgentEvent::Completed:
if let Some(TaskPhase::Verify) = task.phase {
    self.handle_verify_failure(&task_id, &agent);
}
```

```rust
fn handle_verify_failure(&mut self, verify_task_id: &str, _agent: &AgentType) {
    let task_mgr = TaskManager::new(&self.forge_dir);

    let verify_task = match task_mgr.get_task(verify_task_id) {
        Ok(t) => t,
        Err(_) => return,
    };

    let parent_id = match &verify_task.parent_task {
        Some(id) => id.clone(),
        None => return,
    };

    // Check retry count
    if verify_task.retry_count >= 3 {
        self.push_event(&format!(
            "{} failed after 3 retries — needs human attention",
            verify_task_id
        ));
        return;
    }

    // Generate fix subtask
    let next_num = match task_mgr.next_task_number() {
        Ok(n) => n,
        Err(_) => return,
    };

    let fix_id = format!("T-{next_num:03}");
    let retry = verify_task.retry_count + 1;

    // Get the last N lines of agent output as error context
    let error_context: String = self.agent_outputs
        .values()
        .flat_map(|buf| buf.iter().rev().take(20))
        .take(30)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    let mut fix_task = Task::new(
        &fix_id,
        format!("Fix: {} (retry {}/3)", verify_task.title.trim_start_matches("Verify: "), retry),
        format!(
            "Fix the test failures found in verify task {}.\n\
             Parent build task: {}\n\n\
             ## Error Context (last lines of agent output)\n\
             ```\n{}\n```\n\n\
             Fix the issues and ensure all tests pass.",
            verify_task_id, parent_id, error_context
        ),
    );
    fix_task.parent_task = Some(parent_id.clone());
    fix_task.phase = Some(TaskPhase::Fix);
    fix_task.task_type = Some("implement".to_string());
    fix_task.depends_on = vec![verify_task_id.to_string()];
    fix_task.assigned_to = verify_task.assigned_to.clone();
    fix_task.locked_files = verify_task.locked_files.clone();

    if task_mgr.create_task(&fix_task).is_ok() {
        self.push_event(&format!(
            "Auto-generated {} to fix failures (retry {}/3)", fix_id, retry
        ));

        // Also create a re-verify subtask that depends on the fix
        let next_v = task_mgr.next_verify_number().unwrap_or(1);
        let reverify_id = format!("V-{next_v:03}");
        let mut reverify = Task::new(
            &reverify_id,
            format!("Verify: {} (retry {}/3)", verify_task.title.trim_start_matches("Verify: "), retry),
            verify_task.description.clone(),
        );
        reverify.parent_task = Some(parent_id);
        reverify.phase = Some(TaskPhase::Verify);
        reverify.task_type = Some("review".to_string());
        reverify.depends_on = vec![fix_id.clone()];
        reverify.assigned_to = verify_task.assigned_to.clone();
        reverify.locked_files = verify_task.locked_files.clone();
        reverify.acceptance_criteria = verify_task.acceptance_criteria.clone();
        reverify.retry_count = retry;

        task_mgr.create_task(&reverify).ok();
        self.reload_tasks().ok();
    }
}
```

## Part 5: Show phase in dashboard UI (`src/tui/ui.rs`)

Add a phase indicator to the dashboard header. In the title/header section of `render_task_board()` or the top bar:

```rust
// Instead of just "FORGE DASHBOARD", show:
// "FORGE DASHBOARD — Phase: BUILD (12/17)"  or
// "FORGE DASHBOARD — Phase: VERIFY (3/5)"

let phase_label = match app.phase {
    DashboardPhase::Build => {
        let total = app.tasks.iter().filter(|t| t.phase.is_none() || t.phase == Some(TaskPhase::Build)).count();
        let done = app.tasks.iter().filter(|t| {
            (t.phase.is_none() || t.phase == Some(TaskPhase::Build))
            && t.status == TaskStatus::Completed
        }).count();
        format!("BUILD ({done}/{total})")
    }
    DashboardPhase::Verify => {
        let total = app.tasks.iter().filter(|t| t.phase == Some(TaskPhase::Verify)).count();
        let done = app.tasks.iter().filter(|t| t.phase == Some(TaskPhase::Verify) && t.status == TaskStatus::Completed).count();
        format!("VERIFY ({done}/{total})")
    }
    DashboardPhase::Complete => "COMPLETE".to_string(),
};
```

## Part 6: Hierarchical task display in task board

When rendering the task table, indent verify and fix subtasks under their parent:

```
T-001  build  ✓ Done   claude  design    Design Lane C ControlEngine
T-002  build  ✓ Done   codex   implement Implement Lane C ControlEngine
 V-001 verify ✓ Pass   codex   review    Verify: Implement Lane C ControlEngine
T-003  build  ✓ Done   codex   implement Implement RAG Injection
 V-002 verify ✗ Fail   codex   review    Verify: Implement RAG Injection
 T-018 fix    ● Run    codex   implement Fix: Implement RAG Injection (retry 1/3)
 V-003 verify ⏳ Wait  codex   review    Verify: Implement RAG Injection (retry 1/3)
```

To achieve this, sort tasks so children appear immediately after their parent. Group by parent_task:

```rust
fn sort_tasks_hierarchical(tasks: &[Task]) -> Vec<&Task> {
    let mut result: Vec<&Task> = Vec::new();
    let mut children: HashMap<String, Vec<&Task>> = HashMap::new();

    // Separate parents and children
    for task in tasks {
        if let Some(ref parent) = task.parent_task {
            children.entry(parent.clone()).or_default().push(task);
        }
    }

    // Iterate parent tasks (no parent_task field) in order
    for task in tasks {
        if task.parent_task.is_none() {
            result.push(task);
            // Add children immediately after parent
            if let Some(kids) = children.get(&task.id) {
                for child in kids {
                    result.push(child);
                }
            }
        }
    }

    result
}
```

## Files to modify

1. `src/tui/app.rs` — Add `DashboardPhase`, phase tracking, `check_phase_transition()`, `handle_verify_failure()`, test/fix loop
2. `src/tui/ui.rs` — Phase indicator in header, hierarchical task display
3. `src/cli/status.rs` — Hierarchical display (reuse pattern from ui.rs)

## Tests

- Phase starts as `Build`
- Phase transitions to `Verify` when all build tasks complete
- Phase transitions to `Complete` when all verify tasks complete
- Verify failure generates fix + re-verify subtask pair
- Fix/re-verify chain stops after 3 retries
- `retry_count` increments correctly on each re-verify
- Hierarchical sort puts children after parents
- Phase indicator shows correct counts

## Verification

- [ ] `cargo test` — all pass
- [ ] `cargo clippy -- -D warnings` — 0 warnings
- [ ] Dashboard shows phase transitions in event log
- [ ] Verify failures trigger auto-fix (visible in task board)
