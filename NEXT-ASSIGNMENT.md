# Next Assignment: DX-010 — Status Shows Full Task Table with Dependencies

> Priority: HIGH | Estimated: 30-45m | Version target: v0.3.8

## The Problem

`forge status` shows only a summary line (`Total: 17 Pending: 17 Active: 0 Done: 0 Failed: 0`) and a progress bar. No task table, no dependency info, no way to see which tasks are blocked vs ready to run.

## The Fix

Replace the minimal "Task Summary" + "Active Tasks" sections with a full task table showing every task, its status, agent, dependencies, and whether it's blocked or runnable.

### Part 1: Full task table in `src/cli/status.rs`

Replace lines 97-137 (Task Summary + Active Tasks sections) with a full table:

```rust
// ── Task Board ──────────────────────────────────────────────
println!("  {}", "Task Board:".bold());
println!(
    "    {:<8} {:<10} {:<10} {:<8} {:<34} {}",
    "ID".dimmed(),
    "Status".dimmed(),
    "Agent".dimmed(),
    "Type".dimmed(),
    "Title".dimmed(),
    "Dependencies".dimmed(),
);
println!("    {}", "─".repeat(90));

let completed_ids: Vec<String> = tasks
    .iter()
    .filter(|t| t.status == TaskStatus::Completed)
    .map(|t| t.id.clone())
    .collect();

for task in &tasks {
    let status_str = match task.status {
        TaskStatus::Pending => {
            if task.is_blocked(&completed_ids) {
                "⏳ Blocked".yellow().to_string()
            } else {
                "○ Ready".white().to_string()
            }
        }
        TaskStatus::Assigned | TaskStatus::InProgress => "⚡ Running".cyan().to_string(),
        TaskStatus::Completed => "✓ Done".green().to_string(),
        TaskStatus::Failed => "✗ Failed".red().to_string(),
        TaskStatus::Blocked => "⏳ Blocked".yellow().to_string(),
    };

    let agent_str = task
        .assigned_to
        .as_ref()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "-".into());

    let type_str = task.task_type.as_deref().unwrap_or("-");

    let title = if task.title.len() > 32 {
        format!("{}...", &task.title[..29])
    } else {
        task.title.clone()
    };

    let deps = if task.depends_on.is_empty() {
        "-".to_string()
    } else {
        // Color dependencies based on their completion status
        task.depends_on
            .iter()
            .map(|d| {
                if completed_ids.contains(d) {
                    d.green().to_string()
                } else {
                    d.yellow().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    println!(
        "    {:<8} {:<10} {:<10} {:<8} {:<34} {}",
        task.id.cyan(),
        status_str,
        agent_str,
        type_str.dimmed(),
        title,
        deps
    );
}

println!();
```

### Part 2: Keep the summary + progress bar AFTER the table

Move the summary counts and progress bar below the table as a compact footer:

```rust
println!(
    "    {} total | {} ready | {} running | {} done | {} failed | {} blocked",
    total.to_string().bold(),
    ready_count.to_string().white(),
    in_progress.to_string().cyan(),
    completed.to_string().green(),
    failed.to_string().red(),
    blocked_count.to_string().yellow(),
);

// Progress bar
let bar = format!(
    "    [{}{}] {}%",
    "█".repeat(filled).green(),
    "░".repeat(empty),
    progress
);
println!("{bar}");
println!();
```

Where `ready_count` = pending tasks that are NOT blocked, and `blocked_count` = pending tasks that ARE blocked.

### Part 3: Add `--table` and `--summary` flags (optional, nice-to-have)

If time permits, add CLI flags:
- `forge status` — full table (new default)
- `forge status --summary` — just the summary line + progress bar (old behavior)

This is optional. The full table should be the default.

## Files to modify

1. `src/cli/status.rs` — Replace task summary with full table, add ready/blocked counts

## Tests

- Status output includes all task IDs from the task board
- Blocked tasks show "Blocked" when dependencies are incomplete
- Ready tasks show "Ready" when all dependencies are met (or no dependencies)
- Completed dependencies render differently from pending ones
- Empty task board shows appropriate message
- Task title truncation at 32 chars

## Verification

- [ ] `cargo test` — all pass
- [ ] `cargo clippy -- -D warnings` — 0 warnings
- [ ] `forge-orca status` in a project with tasks shows full table
- [ ] Blocked vs Ready distinction is visible
- [ ] Dependencies are color-coded (green = done, yellow = pending)
