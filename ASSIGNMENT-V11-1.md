# Assignment V1.1-1: Task Model Extensions + `forge verify` Command

> Priority: HIGH | Estimated: 30-45min | Version target: v1.1.0

## The Problem

Forge v1.0 has one phase: BUILD. Tasks are created, agents execute them, done. v1.1 adds Phase 2 (VERIFY) — after build tasks complete, forge auto-generates verify subtasks that run tests. This assignment adds the data model foundations and a `forge verify` command.

## Part 1: Extend Task struct (`src/core/task.rs`)

Add 3 new optional fields to the `Task` struct:

```rust
/// Parent task ID for subtasks (e.g., V-002's parent is T-002)
#[serde(default, skip_serializing_if = "Option::is_none")]
pub parent_task: Option<String>,

/// Lifecycle phase: build, verify, fix
#[serde(default, skip_serializing_if = "Option::is_none")]
pub phase: Option<TaskPhase>,

/// Retry count for verify/fix loops (max 3 before flagging for human)
#[serde(default)]
pub retry_count: u32,
```

Add a new `TaskPhase` enum (above or near `TaskStatus`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Build,
    Verify,
    Fix,
}

impl std::fmt::Display for TaskPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskPhase::Build => write!(f, "build"),
            TaskPhase::Verify => write!(f, "verify"),
            TaskPhase::Fix => write!(f, "fix"),
        }
    }
}
```

Update `Task::new()` to initialize the new fields:
```rust
parent_task: None,
phase: None,   // None = legacy task, treated as Build
retry_count: 0,
```

Update `Task::write_to_file()` to include phase and parent info in the markdown output.

## Part 2: Add `next_verify_number()` to TaskManager

Verify tasks use a `V-NNN` ID scheme. Add a method parallel to `next_task_number()`:

```rust
/// Find the highest existing verify task ID number and return the next one.
pub fn next_verify_number(&self) -> anyhow::Result<u32> {
    let tasks_dir = self.forge_dir.join("tasks");
    if !tasks_dir.exists() {
        return Ok(1);
    }

    let mut max_id: u32 = 0;
    for entry in std::fs::read_dir(&tasks_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(num_str) = name
            .strip_prefix("V-")
            .and_then(|s| s.strip_suffix(".json"))
            && let Ok(num) = num_str.parse::<u32>()
        {
            max_id = max_id.max(num);
        }
    }

    Ok(max_id + 1)
}
```

## Part 3: Add `generate_verify_subtasks()` to TaskManager

```rust
/// Generate verify subtasks for all completed build/implement tasks
/// that don't already have a verify subtask.
pub fn generate_verify_subtasks(&self) -> anyhow::Result<Vec<Task>> {
    let tasks = self.list_tasks()?;
    let mut next_v = self.next_verify_number()?;
    let mut generated = Vec::new();

    for task in &tasks {
        // Only verify completed implement/test tasks from build phase
        if task.status != TaskStatus::Completed {
            continue;
        }
        let task_type = task.task_type.as_deref().unwrap_or("");
        if !matches!(task_type, "implement" | "test" | "") {
            continue;
        }
        // Skip if already has a verify subtask
        let has_verify = tasks.iter().any(|t| {
            t.parent_task.as_deref() == Some(&task.id)
                && t.phase == Some(TaskPhase::Verify)
        });
        if has_verify {
            continue;
        }

        let verify_id = format!("V-{next_v:03}");
        let mut verify_task = Task::new(
            &verify_id,
            format!("Verify: {}", task.title),
            format!(
                "Run automated tests for task {}.\n\
                 Review the code changes and verify acceptance criteria are met.\n\
                 Original task: {}\n\
                 Acceptance criteria from parent:\n{}",
                task.id,
                task.description,
                task.acceptance_criteria
                    .iter()
                    .map(|c| format!("- {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        );
        verify_task.parent_task = Some(task.id.clone());
        verify_task.phase = Some(TaskPhase::Verify);
        verify_task.task_type = Some("review".to_string());
        verify_task.depends_on = vec![task.id.clone()];
        verify_task.locked_files = task.locked_files.clone();
        verify_task.acceptance_criteria = task.acceptance_criteria.clone();

        // Assign agent: codex for code, gemini for docs, claude for design
        verify_task.assigned_to = Some(match task_type {
            "document" => AgentType::Gemini,
            "design" => AgentType::Claude,
            _ => AgentType::Codex,
        });

        self.create_task(&verify_task)?;
        generated.push(verify_task);
        next_v += 1;
    }

    Ok(generated)
}
```

## Part 4: New CLI command `forge verify` (`src/cli/verify.rs`)

Create a new file `src/cli/verify.rs`:

```rust
use crate::core::task::TaskManager;
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        println!("{} Forge is not initialized. Run {} first.", "!".yellow(), "forge init".cyan());
        return Ok(());
    }

    let task_mgr = TaskManager::new(&forge_dir);
    let generated = task_mgr.generate_verify_subtasks()?;

    if generated.is_empty() {
        println!("{} No new verify tasks to generate.", "✓".green());
        println!("  All completed build tasks already have verify subtasks,");
        println!("  or no build tasks are completed yet.");
    } else {
        println!();
        println!("{}", "Verify subtasks generated:".bold());
        for task in &generated {
            let parent = task.parent_task.as_deref().unwrap_or("?");
            let agent = task.assigned_to.as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "auto".into());
            println!("  {} {} [{}] ← parent {}", "○".cyan(), task.id, agent, parent);
        }
        println!();
        println!("  {} verify tasks created. Run {} or {} to execute.",
            generated.len(),
            "forge dashboard".cyan(),
            "forge run".cyan(),
        );
    }

    Ok(())
}
```

## Part 5: Wire into CLI (`src/cli/mod.rs` + `src/main.rs`)

In `src/cli/mod.rs`, add:
```rust
pub mod verify;
```

Add to the `Commands` enum:
```rust
/// Generate verify subtasks for completed build tasks (Phase 2)
Verify,
```

In `src/main.rs`, add the dispatch:
```rust
Commands::Verify => {
    cli::verify::execute(&project_root)?;
}
```

## Part 6: Update `forge status` to show phase

In `src/cli/status.rs`, when rendering the task table, show the phase column if any tasks have a phase set. Show parent relationship with indentation:

```
ID      Phase  Status  Agent   Type      Title
T-001   build  Done    claude  design    Design Lane C ControlEngine
T-002   build  Done    codex   implement Implement Lane C ControlEngine
 V-001  verify ○ Ready codex   review    Verify: Implement Lane C ControlEngine
T-003   build  Done    gemini  document  Document Lane C
 V-002  verify ○ Ready gemini  review    Verify: Document Lane C
```

The indentation (space before V-xxx) and the `←` or parent column are optional nice-to-haves.

## Files to modify

1. `src/core/task.rs` — Add `TaskPhase`, `parent_task`, `phase`, `retry_count` fields + `next_verify_number()` + `generate_verify_subtasks()`
2. `src/cli/verify.rs` — New file: `forge verify` command
3. `src/cli/mod.rs` — Add `verify` module + `Verify` command variant
4. `src/main.rs` — Route `Commands::Verify`
5. `src/cli/status.rs` — Show phase column and parent indentation

## Tests

- `TaskPhase` serializes/deserializes correctly (round-trip)
- `parent_task` field persists through JSON write/read
- `next_verify_number()` works like `next_task_number()` but for V-prefix
- `generate_verify_subtasks()` creates verify tasks for completed implement tasks
- `generate_verify_subtasks()` skips tasks that already have verify subtasks (idempotent)
- `generate_verify_subtasks()` skips non-implement tasks (design, document)
- `generate_verify_subtasks()` assigns correct agents (codex for implement, gemini for document)
- `retry_count` defaults to 0 and persists
- CLI `forge verify` works with no args

## Verification

- [ ] `cargo test` — all pass
- [ ] `cargo clippy -- -D warnings` — 0 warnings
- [ ] `forge-orca verify` generates verify subtasks
- [ ] `forge-orca status` shows phase column and verify subtasks
