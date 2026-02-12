# Assignment: v1.1.0 — "The Verifier"

> **Scope:** 4 parts, implement sequentially. Run `cargo test` and `cargo clippy -- -D warnings` after EACH part before moving to the next.
> **Version target:** v1.1.0 in Cargo.toml

## Overview

Forge v1.0 has one phase: BUILD. v1.1 adds Phase 2 (VERIFY — automated testing) and Phase 3 (HUMAN UAT — finding capture). The full lifecycle becomes:

```
forge plan --generate          # Create build tasks
forge dashboard                # Phase 1: BUILD → Phase 2: VERIFY (auto)
forge uat                      # Phase 3: Human UAT (interactive finding capture)
forge plan --from-findings     # Generate fix tasks from UAT findings
forge dashboard                # Fix cycle
```

---

## PART 1: Task Model Extensions + `forge verify` Command

### 1a: Extend Task struct (`src/core/task.rs`)

Add `TaskPhase` enum near `TaskStatus`:

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

Add 3 fields to `Task`:

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

Initialize in `Task::new()`: `parent_task: None, phase: None, retry_count: 0`.

Update `Task::write_to_file()` to include phase and parent in markdown.

### 1b: Add `next_verify_number()` to TaskManager

Same pattern as `next_task_number()` but scans `V-NNN.json` files:

```rust
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

### 1c: Add `generate_verify_subtasks()` to TaskManager

For each completed `implement` or `test` task that doesn't already have a verify subtask, generate one:

```rust
pub fn generate_verify_subtasks(&self) -> anyhow::Result<Vec<Task>> {
    let tasks = self.list_tasks()?;
    let mut next_v = self.next_verify_number()?;
    let mut generated = Vec::new();

    for task in &tasks {
        if task.status != TaskStatus::Completed { continue; }
        let task_type = task.task_type.as_deref().unwrap_or("");
        if !matches!(task_type, "implement" | "test" | "") { continue; }

        // Skip if already has a verify subtask
        let has_verify = tasks.iter().any(|t| {
            t.parent_task.as_deref() == Some(&task.id)
                && t.phase == Some(TaskPhase::Verify)
        });
        if has_verify { continue; }

        let verify_id = format!("V-{next_v:03}");
        let mut verify_task = Task::new(
            &verify_id,
            format!("Verify: {}", task.title),
            format!(
                "Run automated tests for task {}.\nReview changes and verify acceptance criteria.\nOriginal: {}\nCriteria:\n{}",
                task.id, task.description,
                task.acceptance_criteria.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")
            ),
        );
        verify_task.parent_task = Some(task.id.clone());
        verify_task.phase = Some(TaskPhase::Verify);
        verify_task.task_type = Some("review".to_string());
        verify_task.depends_on = vec![task.id.clone()];
        verify_task.locked_files = task.locked_files.clone();
        verify_task.acceptance_criteria = task.acceptance_criteria.clone();
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

### 1d: New CLI command `forge verify`

Create `src/cli/verify.rs`:

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
    } else {
        println!();
        println!("{}", "Verify subtasks generated:".bold());
        for task in &generated {
            let parent = task.parent_task.as_deref().unwrap_or("?");
            let agent = task.assigned_to.as_ref().map(|a| a.to_string()).unwrap_or_else(|| "auto".into());
            println!("  {} {} [{}] ← parent {}", "○".cyan(), task.id, agent, parent);
        }
        println!();
        println!("  {} verify tasks created. Run {} or {} to execute.",
            generated.len(), "forge dashboard".cyan(), "forge run".cyan());
    }
    Ok(())
}
```

Wire into `src/cli/mod.rs` (`pub mod verify;` + `Verify` variant) and `src/main.rs`.

### 1e: Update status table for phase + hierarchy

In `src/cli/status.rs`, add a phase column and indent subtasks (V-xxx, fix tasks) under their parent with a leading space.

### 1f: Tests for Part 1

- `TaskPhase` round-trip serialization
- `parent_task` persists through JSON
- `next_verify_number()` works like `next_task_number()` for V-prefix
- `generate_verify_subtasks()` creates verify tasks for completed implement tasks
- `generate_verify_subtasks()` is idempotent (skips existing verify subtasks)
- `generate_verify_subtasks()` assigns correct agents per task type
- `retry_count` defaults to 0 and persists
- CLI: `forge verify` succeeds with no tasks, succeeds with completed tasks

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass before continuing.**

---

## PART 2: Dashboard Phase 2 Auto-Transition + Test/Fix Loop

### 2a: Phase tracking in App (`src/tui/app.rs`)

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPhase {
    Build,
    Verify,
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

Add `pub phase: DashboardPhase` to `App`, init as `Build`.

### 2b: Auto-transition from Build to Verify

After any task completion, check if all build-phase tasks (phase is None or Build) are done. If so:
1. Set `phase = Verify`
2. Call `task_mgr.generate_verify_subtasks()`
3. Reload tasks
4. Push event "Phase 1 (BUILD) complete — transitioning to VERIFY"

### 2c: Test/Fix loop on verify failure

When a verify task (phase == Verify) fails:
1. Check `retry_count < 3`
2. Generate a fix subtask (T-xxx, phase=Fix, type=implement) that depends on the failed verify task
3. Generate a re-verify subtask (V-xxx, phase=Verify, retry_count=retry+1) that depends on the fix task
4. Push event "Auto-generated T-xxx to fix failures (retry N/3)"
5. If retry_count >= 3, push event "V-xxx failed after 3 retries — needs human attention"

### 2d: Phase-aware completion detection

Update `all_complete` logic:
- In Build phase: when all build tasks done → transition to Verify (don't mark complete)
- In Verify phase: when all verify+fix tasks done → set phase=Complete, all_complete=true
- Show phase-aware completion banner

### 2e: Phase indicator in UI (`src/tui/ui.rs`)

Show phase in header: `"FORGE DASHBOARD — BUILD (12/17)"` or `"FORGE DASHBOARD — VERIFY (3/5)"`.

### 2f: Hierarchical task display

Sort tasks so children appear after parents. Indent subtask IDs with a leading space. Reuse this sort in both `ui.rs` (dashboard) and `status.rs` (CLI).

### 2g: Tests for Part 2

- Phase starts as Build
- Phase transitions to Verify when all build tasks complete
- Phase transitions to Complete when all verify tasks complete
- Verify failure generates fix + re-verify pair
- Fix/re-verify stops after 3 retries
- Hierarchical sort puts children after parents

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass before continuing.**

---

## PART 3: Finding Model + `forge uat` Command

### 3a: Finding model (`src/core/finding.rs` — new file)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Positive,
}

impl std::fmt::Display for FindingSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingSeverity::Critical => write!(f, "critical"),
            FindingSeverity::High => write!(f, "high"),
            FindingSeverity::Medium => write!(f, "medium"),
            FindingSeverity::Low => write!(f, "low"),
            FindingSeverity::Positive => write!(f, "positive"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FindingType {
    Bug,
    Missing,
    Enhancement,
    Positive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub description: String,
    pub severity: FindingSeverity,
    pub finding_type: FindingType,
    pub related_tasks: Vec<String>,
    pub created_at: DateTime<Utc>,
}

pub struct FindingManager {
    forge_dir: PathBuf,
}

impl FindingManager {
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self { forge_dir: forge_dir.into() }
    }

    fn findings_dir(&self) -> PathBuf {
        self.forge_dir.join("findings")
    }

    pub fn save_finding(&self, finding: &Finding) -> anyhow::Result<()> {
        let dir = self.findings_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", finding.id));
        let content = serde_json::to_string_pretty(finding)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn list_findings(&self) -> anyhow::Result<Vec<Finding>> {
        let dir = self.findings_dir();
        if !dir.exists() { return Ok(Vec::new()); }
        let mut findings = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = std::fs::read_to_string(&path)?;
                if let Ok(finding) = serde_json::from_str::<Finding>(&content) {
                    findings.push(finding);
                }
            }
        }
        findings.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(findings)
    }

    pub fn next_finding_number(&self) -> anyhow::Result<u32> {
        let dir = self.findings_dir();
        if !dir.exists() { return Ok(1); }
        let mut max_id: u32 = 0;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name
                .strip_prefix("F-")
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(num) = num_str.parse::<u32>()
            {
                max_id = max_id.max(num);
            }
        }
        Ok(max_id + 1)
    }
}
```

Register in `src/core/mod.rs`: `pub mod finding;`

### 3b: Classification heuristics

Add a classify function (keyword-based, no LLM needed):

```rust
/// Classify a finding description into severity and type using keyword heuristics.
pub fn classify_finding(description: &str) -> (FindingSeverity, FindingType) {
    let lower = description.to_lowercase();

    let severity = if lower.contains("crash") || lower.contains("data loss") || lower.contains("security") {
        FindingSeverity::Critical
    } else if lower.contains("broken") || lower.contains("doesn't work") || lower.contains("fail") || lower.contains("error") {
        FindingSeverity::High
    } else if lower.contains("slow") || lower.contains("confusing") || lower.contains("unclear") || lower.contains("should") {
        FindingSeverity::Medium
    } else if lower.contains("love") || lower.contains("great") || lower.contains("fast") || lower.contains("nice") || lower.contains("excellent") {
        FindingSeverity::Positive
    } else {
        FindingSeverity::Low
    };

    let finding_type = if matches!(severity, FindingSeverity::Positive) {
        FindingType::Positive
    } else if lower.contains("missing") || lower.contains("need") || lower.contains("should have") || lower.contains("add") {
        FindingType::Missing
    } else if lower.contains("improve") || lower.contains("better") || lower.contains("enhance") {
        FindingType::Enhancement
    } else {
        FindingType::Bug
    };

    (severity, finding_type)
}

/// Find related tasks by matching keywords from the description against task titles.
pub fn find_related_tasks(description: &str, tasks: &[crate::core::task::Task]) -> Vec<String> {
    let lower = description.to_lowercase();
    tasks.iter()
        .filter(|t| {
            let title_lower = t.title.to_lowercase();
            // Check if any significant word from the description appears in the task title
            lower.split_whitespace()
                .filter(|w| w.len() > 3) // skip short words
                .any(|w| title_lower.contains(w))
        })
        .map(|t| t.id.clone())
        .collect()
}
```

### 3c: `forge uat` CLI command (`src/cli/uat.rs` — new file)

Interactive REPL that captures UAT findings:

```rust
use crate::core::finding::{Finding, FindingManager, classify_finding, find_related_tasks};
use crate::core::task::TaskManager;
use colored::Colorize;
use std::io::{self, BufRead, Write};
use std::path::Path;

pub fn execute(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        println!("{} Forge is not initialized.", "!".yellow());
        return Ok(());
    }

    let task_mgr = TaskManager::new(&forge_dir);
    let finding_mgr = FindingManager::new(&forge_dir);
    let tasks = task_mgr.list_tasks()?;

    println!();
    println!("{}", "UAT Mode — Describe issues naturally. Type 'done' when finished.".bold());
    println!();

    // Show acceptance criteria from completed tasks
    let criteria: Vec<_> = tasks.iter()
        .filter(|t| t.status == crate::core::task::TaskStatus::Completed)
        .flat_map(|t| t.acceptance_criteria.iter().map(move |c| (t.id.clone(), c.clone())))
        .collect();

    if !criteria.is_empty() {
        println!("  {}:", "Acceptance Criteria (from completed tasks)".dimmed());
        for (task_id, criterion) in &criteria {
            println!("  {} {} ({})", "[ ]".dimmed(), criterion, task_id.dimmed());
        }
        println!();
    }

    let mut findings: Vec<Finding> = Vec::new();
    let mut next_num = finding_mgr.next_finding_number()?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("  {} ", ">".cyan());
        stdout.flush()?;

        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() { continue; }
        if input.eq_ignore_ascii_case("done") || input.eq_ignore_ascii_case("quit") || input.eq_ignore_ascii_case("exit") {
            break;
        }

        let (severity, finding_type) = classify_finding(input);
        let related = find_related_tasks(input, &tasks);

        let finding = Finding {
            id: format!("F-{next_num:03}"),
            description: input.to_string(),
            severity: severity.clone(),
            finding_type: finding_type.clone(),
            related_tasks: related.clone(),
            created_at: chrono::Utc::now(),
        };

        finding_mgr.save_finding(&finding)?;
        findings.push(finding);
        next_num += 1;

        let related_str = if related.is_empty() {
            String::new()
        } else {
            format!(", related: {}", related.join(", "))
        };

        let severity_colored = match severity {
            crate::core::finding::FindingSeverity::Critical => "critical".red().bold().to_string(),
            crate::core::finding::FindingSeverity::High => "high".red().to_string(),
            crate::core::finding::FindingSeverity::Medium => "medium".yellow().to_string(),
            crate::core::finding::FindingSeverity::Low => "low".dimmed().to_string(),
            crate::core::finding::FindingSeverity::Positive => "positive".green().to_string(),
        };

        println!("  {} (severity: {}{}) ",
            "Captured".green(),
            severity_colored,
            related_str,
        );
    }

    // Summary
    println!();
    let bugs = findings.iter().filter(|f| !matches!(f.severity, crate::core::finding::FindingSeverity::Positive)).count();
    let positives = findings.iter().filter(|f| matches!(f.severity, crate::core::finding::FindingSeverity::Positive)).count();
    println!("  {} findings captured ({} issues, {} positive).", findings.len(), bugs, positives);

    if bugs > 0 {
        println!("  Run {} to generate fix tasks.", "forge plan --from-findings".cyan());
    }
    println!();

    Ok(())
}
```

Wire into `src/cli/mod.rs` (`pub mod uat;` + `Uat` variant) and `src/main.rs`.

### 3d: Tests for Part 3

- Finding serialization round-trip
- `classify_finding()` returns correct severity for keywords: "crash" → Critical, "broken" → High, "slow" → Medium, "love" → Positive
- `classify_finding()` returns correct type: "missing" → Missing, "improve" → Enhancement, default → Bug
- `find_related_tasks()` matches keywords against task titles
- `FindingManager` save/list round-trip
- `next_finding_number()` monotonic IDs

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass before continuing.**

---

## PART 4: `forge plan --from-findings`

### 4a: Add `--from-findings` flag to plan command

In `src/cli/mod.rs`, add to the `Plan` variant:

```rust
Plan {
    #[arg(short, long)]
    generate: bool,

    #[arg(short, long)]
    spec: Option<String>,

    /// Generate fix tasks from UAT findings (.forge/findings/)
    #[arg(long)]
    from_findings: bool,
},
```

### 4b: Implement finding-to-task generation in `src/cli/plan.rs`

Add a new function that reads findings and generates fix tasks:

```rust
pub fn generate_from_findings(project_root: &Path) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    let finding_mgr = FindingManager::new(&forge_dir);
    let task_mgr = TaskManager::new(&forge_dir);

    let findings = finding_mgr.list_findings()?;
    let actionable: Vec<_> = findings.iter()
        .filter(|f| !matches!(f.severity, FindingSeverity::Positive))
        .collect();

    if actionable.is_empty() {
        println!("{} No actionable findings. Run {} first.", "!".yellow(), "forge uat".cyan());
        return Ok(());
    }

    let mut next_num = task_mgr.next_task_number()?;
    let plan_version = /* determine current plan version */ 0; // scan existing tasks for max plan_version + 1
    let mut generated = Vec::new();

    for finding in &actionable {
        let task_type = match finding.finding_type {
            FindingType::Bug => "implement",
            FindingType::Missing => "implement",
            FindingType::Enhancement => "implement",
            FindingType::Positive => continue,
        };

        let priority = match finding.severity {
            FindingSeverity::Critical => "P0",
            FindingSeverity::High => "P1",
            FindingSeverity::Medium => "P2",
            FindingSeverity::Low => "P3",
            FindingSeverity::Positive => continue,
        };

        let task_id = format!("T-{next_num:03}");
        let mut task = Task::new(
            &task_id,
            format!("Fix: {}", truncate(&finding.description, 60)),
            format!(
                "UAT Finding {}: {}\n\nSeverity: {}\nPriority: {}\nRelated tasks: {}\n\nFix this issue.",
                finding.id,
                finding.description,
                finding.severity,
                priority,
                if finding.related_tasks.is_empty() { "none".to_string() }
                else { finding.related_tasks.join(", ") }
            ),
        );
        task.task_type = Some(task_type.to_string());
        task.phase = Some(TaskPhase::Fix);
        task.assigned_to = Some(AgentType::Claude); // Claude for bug fixes
        task.plan_version = Some(plan_version);

        // If finding references specific tasks, depend on them
        // (they should already be complete, so this is just traceability)

        task_mgr.create_task(&task)?;
        generated.push((task_id, finding));
        next_num += 1;
    }

    println!();
    println!("{}", "Fix tasks generated from UAT findings:".bold());
    for (task_id, finding) in &generated {
        let sev = match finding.severity {
            FindingSeverity::Critical => "CRIT".red().bold().to_string(),
            FindingSeverity::High => "HIGH".red().to_string(),
            FindingSeverity::Medium => "MED".yellow().to_string(),
            _ => "LOW".dimmed().to_string(),
        };
        println!("  {} {} [{}] {}", "○".cyan(), task_id, sev, truncate(&finding.description, 50));
    }
    println!();
    println!("  {} fix tasks created. Run {} or {} to execute.",
        generated.len(), "forge dashboard".cyan(), "forge run".cyan());
    println!();

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max.saturating_sub(3)]) }
}
```

### 4c: Wire into plan.rs dispatch

In the `execute()` function of `plan.rs`, handle the new flag:

```rust
pub fn execute(
    project_root: &Path,
    generate: bool,
    spec_path: Option<String>,
    from_findings: bool,
) -> anyhow::Result<()> {
    if from_findings {
        return generate_from_findings(project_root);
    }
    // ... existing generate logic
}
```

Update `src/main.rs` to pass `from_findings` to `cli::plan::execute()`.

### 4d: Tests for Part 4

- `--from-findings` generates tasks from findings
- Positive findings are skipped
- Task IDs are monotonic (appended after existing tasks)
- Generated tasks have phase=Fix and correct severity-based priority
- Empty findings directory produces helpful message
- Finding-to-task severity mapping is correct

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass before continuing.**

---

## Final Steps

After all 4 parts pass tests and clippy:

1. Bump version in `Cargo.toml` to `1.1.0`
2. Update CHANGELOG.md — add a `[1.1.0]` section with all changes
3. Run final `cargo test && cargo clippy -- -D warnings` to confirm everything

## Files Summary

| File | Action | Part |
|------|--------|------|
| `src/core/task.rs` | Modify — add TaskPhase, parent_task, phase, retry_count, next_verify_number, generate_verify_subtasks | 1 |
| `src/core/finding.rs` | **New** — Finding model, FindingManager, classify, relate | 3 |
| `src/core/mod.rs` | Modify — add `pub mod finding` | 3 |
| `src/cli/verify.rs` | **New** — `forge verify` command | 1 |
| `src/cli/uat.rs` | **New** — `forge uat` interactive REPL | 3 |
| `src/cli/mod.rs` | Modify — add Verify, Uat variants + modules | 1, 3 |
| `src/cli/plan.rs` | Modify — add --from-findings + generate_from_findings() | 4 |
| `src/cli/status.rs` | Modify — phase column, hierarchical display | 1 |
| `src/main.rs` | Modify — route Verify, Uat, pass from_findings | 1, 3, 4 |
| `src/tui/app.rs` | Modify — DashboardPhase, auto-transition, test/fix loop | 2 |
| `src/tui/ui.rs` | Modify — phase indicator, hierarchical task display | 2 |
| `Cargo.toml` | Modify — version 1.1.0 | Final |
| `CHANGELOG.md` | Modify — add [1.1.0] section | Final |
