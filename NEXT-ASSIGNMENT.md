# Assignment: DX-032 — Standalone UAT TUI

> **Scope:** 1 part. Run `cargo test && cargo clippy -- -D warnings` when done.
> **Version target:** Stay at v1.1.0 (patch, not bump)

## Context

`forge uat` currently dumps ALL acceptance criteria from ALL completed tasks (including V-xxx verify subtasks) as a massive unreadable wall of text, then shows a bare `>` prompt. This is unusable on real projects — voice-jib-jab had 100+ criteria lines.

## Solution

Replace the current CLI REPL (`src/cli/uat.rs`) with a ratatui-based TUI (like the dashboard). The UAT TUI is a separate, focused interface for human acceptance testing — NOT a new tab in the dashboard.

Also add an **inline one-shot mode**: `forge uat "description"` captures a single finding and exits (no TUI).

## Architecture

### New files:
- `src/tui/uat_app.rs` — UAT app state + event handling
- `src/tui/uat_ui.rs` — UAT rendering

### Modified files:
- `src/cli/mod.rs` — Add `finding` arg to `Uat` command variant
- `src/main.rs` — Route `Uat` with optional finding arg
- `src/cli/uat.rs` — Replace REPL with TUI launch (+ inline mode)
- `src/tui/mod.rs` — Add `pub mod uat_app; pub mod uat_ui;`

---

## Part 1: CLI changes — Add inline mode arg

### `src/cli/mod.rs`

Change the `Uat` variant to accept an optional inline finding:

```rust
/// Interactive UAT — describe issues naturally, capture findings
Uat {
    /// Quick capture: describe a finding inline without opening TUI
    #[arg()]
    finding: Option<String>,
},
```

### `src/main.rs`

Update the match arm:

```rust
Commands::Uat { finding } => {
    cli::uat::execute(&project_root, finding)?;
}
```

### `src/cli/uat.rs`

Replace the entire file. Two paths:

1. **Inline mode** (`forge uat "the button is broken"`) — classify, save finding, print confirmation, exit. No TUI.
2. **TUI mode** (`forge uat`) — launch the ratatui UAT interface.

```rust
use crate::core::finding::{classify_finding, find_related_tasks, Finding, FindingManager};
use crate::core::task::TaskManager;
use colored::Colorize;
use std::path::Path;

pub fn execute(project_root: &Path, inline_finding: Option<String>) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");
    if !forge_dir.exists() {
        println!(
            "{} Forge is not initialized. Run {} first.",
            "!".yellow(),
            "forge init".cyan()
        );
        return Ok(());
    }

    if let Some(description) = inline_finding {
        return execute_inline(&forge_dir, &description);
    }

    // Launch the TUI
    crate::tui::uat_app::run(forge_dir, project_root.to_path_buf())
}

fn execute_inline(forge_dir: &Path, description: &str) -> anyhow::Result<()> {
    let task_mgr = TaskManager::new(forge_dir);
    let finding_mgr = FindingManager::new(forge_dir);
    let tasks = task_mgr.list_tasks()?;

    let (severity, finding_type) = classify_finding(description);
    let related = find_related_tasks(description, &tasks);
    let next_num = finding_mgr.next_finding_number()?;

    let finding = Finding {
        id: format!("F-{next_num:03}"),
        description: description.to_string(),
        severity: severity.clone(),
        finding_type,
        related_tasks: related.clone(),
        created_at: chrono::Utc::now(),
    };

    finding_mgr.save_finding(&finding)?;

    let severity_colored = match &severity {
        crate::core::finding::FindingSeverity::Critical => "critical".red().bold().to_string(),
        crate::core::finding::FindingSeverity::High => "high".red().to_string(),
        crate::core::finding::FindingSeverity::Medium => "medium".yellow().to_string(),
        crate::core::finding::FindingSeverity::Low => "low".dimmed().to_string(),
        crate::core::finding::FindingSeverity::Positive => "positive".green().to_string(),
    };

    let related_str = if related.is_empty() {
        String::new()
    } else {
        format!(", related: {}", related.join(", "))
    };

    println!(
        "  {} {} (severity: {}{})",
        "Captured".green(),
        finding.id.cyan(),
        severity_colored,
        related_str,
    );

    Ok(())
}
```

---

## Part 2: UAT App State (`src/tui/uat_app.rs`)

This is the core state machine for the UAT TUI.

```rust
use crate::core::finding::{classify_finding, find_related_tasks, Finding, FindingManager};
use crate::core::task::{Task, TaskManager, TaskPhase, TaskStatus};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::time::Duration;

/// UAT status for each task
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UatStatus {
    Untested,
    Passed,
    HasFindings,
}

pub struct UatTask {
    pub task: Task,
    pub uat_status: UatStatus,
    pub finding_count: usize,
}

pub struct UatApp {
    pub forge_dir: PathBuf,
    pub project_root: PathBuf,
    pub tasks: Vec<UatTask>,
    pub findings: Vec<Finding>,
    pub selected_task: usize,
    pub input_buffer: String,
    pub input_active: bool,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub finding_scroll: usize,
    pub task_mgr: TaskManager,
    pub finding_mgr: FindingManager,
}

impl UatApp {
    pub fn new(forge_dir: PathBuf, project_root: PathBuf) -> anyhow::Result<Self> {
        let task_mgr = TaskManager::new(&forge_dir);
        let finding_mgr = FindingManager::new(&forge_dir);

        let all_tasks = task_mgr.list_tasks()?;
        let findings = finding_mgr.list_findings()?;

        // Filter: only completed build/fix phase tasks (NOT V-xxx verify subtasks)
        let uat_tasks: Vec<UatTask> = all_tasks
            .into_iter()
            .filter(|t| {
                t.status == TaskStatus::Completed
                    && !matches!(t.phase, Some(TaskPhase::Verify))
            })
            .map(|t| {
                let finding_count = findings
                    .iter()
                    .filter(|f| f.related_tasks.contains(&t.id))
                    .count();
                let uat_status = if finding_count > 0 {
                    UatStatus::HasFindings
                } else {
                    UatStatus::Untested
                };
                UatTask {
                    task: t,
                    uat_status,
                    finding_count,
                }
            })
            .collect();

        Ok(Self {
            forge_dir,
            project_root,
            tasks: uat_tasks,
            findings,
            selected_task: 0,
            input_buffer: String::new(),
            input_active: false,
            should_quit: false,
            status_message: None,
            finding_scroll: 0,
            task_mgr,
            finding_mgr,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C or 'q' always quits (unless typing)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.input_active {
            self.handle_input_key(key);
        } else {
            self.handle_nav_key(key);
        }
    }

    fn handle_nav_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_task > 0 {
                    self.selected_task -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_task + 1 < self.tasks.len() {
                    self.selected_task += 1;
                }
            }
            KeyCode::Char('p') => {
                // Mark selected task as passed
                if let Some(task) = self.tasks.get_mut(self.selected_task) {
                    task.uat_status = UatStatus::Passed;
                    self.status_message =
                        Some(format!("{} marked as UAT passed", task.task.id));
                }
            }
            KeyCode::Char('f') | KeyCode::Enter => {
                // Enter finding input mode
                self.input_active = true;
                self.input_buffer.clear();
                self.status_message = Some("Type finding, press Enter to capture, Esc to cancel".into());
            }
            _ => {}
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_active = false;
                self.input_buffer.clear();
                self.status_message = None;
            }
            KeyCode::Enter => {
                if !self.input_buffer.trim().is_empty() {
                    self.capture_finding();
                }
                self.input_active = false;
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn capture_finding(&mut self) {
        let description = self.input_buffer.trim().to_string();
        if description.is_empty() {
            return;
        }

        let all_tasks: Vec<Task> = self.tasks.iter().map(|t| t.task.clone()).collect();
        let (severity, finding_type) = classify_finding(&description);

        // Auto-relate to the currently selected task
        let mut related = find_related_tasks(&description, &all_tasks);
        if let Some(selected) = self.tasks.get(self.selected_task) {
            if !related.contains(&selected.task.id) {
                related.insert(0, selected.task.id.clone());
            }
        }

        let next_num = self.finding_mgr.next_finding_number().unwrap_or(1);
        let finding = Finding {
            id: format!("F-{next_num:03}"),
            description,
            severity: severity.clone(),
            finding_type,
            related_tasks: related,
            created_at: chrono::Utc::now(),
        };

        if self.finding_mgr.save_finding(&finding).is_ok() {
            self.status_message = Some(format!(
                "Captured {} (severity: {})",
                finding.id, severity
            ));

            // Update the selected task's status
            if let Some(task) = self.tasks.get_mut(self.selected_task) {
                task.uat_status = UatStatus::HasFindings;
                task.finding_count += 1;
            }

            self.findings.push(finding);
        }

        self.input_buffer.clear();
    }

    /// Summary stats for the footer
    pub fn stats(&self) -> (usize, usize, usize, usize) {
        let total = self.tasks.len();
        let passed = self.tasks.iter().filter(|t| t.uat_status == UatStatus::Passed).count();
        let with_findings = self.tasks.iter().filter(|t| t.uat_status == UatStatus::HasFindings).count();
        let untested = self.tasks.iter().filter(|t| t.uat_status == UatStatus::Untested).count();
        (total, passed, with_findings, untested)
    }
}

/// Entry point: launch the UAT TUI
pub fn run(forge_dir: PathBuf, project_root: PathBuf) -> anyhow::Result<()> {
    let mut app = UatApp::new(forge_dir, project_root)?;

    if app.tasks.is_empty() {
        println!("  {} No completed tasks to test.", "!".yellow());
        println!("  Run the dashboard first to complete tasks, then run {} for UAT.", "forge uat".cyan());
        return Ok(());
    }

    // Setup terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Main loop
    loop {
        terminal.draw(|f| crate::tui::uat_ui::render(f, &app))?;

        if crossterm::event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;

    // Print summary on exit
    let (total, passed, with_findings, untested) = app.stats();
    let bugs = app.findings.iter().filter(|f| {
        !matches!(f.severity, crate::core::finding::FindingSeverity::Positive)
    }).count();

    println!();
    println!("  UAT Summary: {} total | {} passed | {} with findings | {} untested",
        total, passed, with_findings, untested);
    println!("  {} findings captured ({} issues).", app.findings.len(), bugs);

    if bugs > 0 {
        println!("  Run {} to generate fix tasks.", "forge plan --from-findings".cyan());
    }
    println!();

    Ok(())
}
```

---

## Part 3: UAT Rendering (`src/tui/uat_ui.rs`)

### Layout

```
┌──────────────────────────────────────────────────────────────┐
│ FORGE UAT — voice-jib-jab — 3/17 tested                     │
├──────────────────────────────┬───────────────────────────────┤
│ Tasks                        │ Criteria & Findings           │
│                              │                               │
│ > ✓ T-001 Fix audio loop     │ Acceptance Criteria:          │
│   ○ T-002 Add stop button    │  [ ] Audio plays without echo │
│   ✗ T-018 Fix: feedback      │  [ ] RMS gate prevents loop   │
│   ○ T-019 Fix: stop btn      │  [ ] Works Chrome/Firefox     │
│                              │                               │
│                              │ Findings for T-001:           │
│                              │  F-006 echo on dual tabs (med)│
│                              │                               │
├──────────────────────────────┴───────────────────────────────┤
│ > type finding here...                                       │
├──────────────────────────────────────────────────────────────┤
│ ↑↓ navigate │ Enter/f capture │ p pass │ q quit              │
└──────────────────────────────────────────────────────────────┘
```

### Rendering function

```rust
use crate::tui::uat_app::{UatApp, UatStatus};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &UatApp) {
    let chunks = Layout::vertical([
        Constraint::Length(3),  // Header
        Constraint::Fill(1),   // Main content (task list + criteria)
        Constraint::Length(3), // Input area
        Constraint::Length(1), // Footer
    ])
    .split(f.area());

    render_header(f, app, chunks[0]);
    render_main(f, app, chunks[1]);
    render_input(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

fn render_header(f: &mut Frame, app: &UatApp, area: Rect) {
    let (total, passed, with_findings, _untested) = app.stats();
    let tested = passed + with_findings;

    // Get project name from state if available
    let project_name = {
        let state_mgr = crate::core::state::StateManager::new(&app.forge_dir);
        state_mgr
            .load()
            .map(|s| s.project_name.clone())
            .unwrap_or_default()
    };

    let title = if project_name.is_empty() {
        format!(" FORGE UAT \u{2014} {tested}/{total} tested ")
    } else {
        format!(" FORGE UAT \u{2014} {project_name} \u{2014} {tested}/{total} tested ")
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);
}

fn render_main(f: &mut Frame, app: &UatApp, area: Rect) {
    let main_chunks = Layout::horizontal([
        Constraint::Percentage(40), // Task list
        Constraint::Percentage(60), // Criteria + findings
    ])
    .split(area);

    render_task_list(f, app, main_chunks[0]);
    render_criteria(f, app, main_chunks[1]);
}

fn render_task_list(f: &mut Frame, app: &UatApp, area: Rect) {
    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let status_icon = match t.uat_status {
                UatStatus::Passed => Span::styled("  \u{2713} ", Style::default().fg(Color::Green)),
                UatStatus::HasFindings => Span::styled("  \u{2717} ", Style::default().fg(Color::Red)),
                UatStatus::Untested => Span::styled("  \u{25CB} ", Style::default().fg(Color::DarkGray)),
            };

            let title = truncate_chars(&t.task.title, 30);

            let id_style = if i == app.selected_task {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let line = Line::from(vec![
                status_icon,
                Span::styled(format!("{} ", t.task.id), id_style),
                Span::raw(title),
            ]);

            if i == app.selected_task {
                ListItem::new(line).style(Style::default().bg(Color::DarkGray))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Tasks ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White)),
    );

    f.render_widget(list, area);
}

fn render_criteria(f: &mut Frame, app: &UatApp, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    if let Some(selected) = app.tasks.get(app.selected_task) {
        // Acceptance criteria
        if !selected.task.acceptance_criteria.is_empty() {
            lines.push(Line::styled(
                " Acceptance Criteria:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));

            for criterion in &selected.task.acceptance_criteria {
                lines.push(Line::from(vec![
                    Span::styled("  [ ] ", Style::default().fg(Color::DarkGray)),
                    Span::raw(criterion.as_str()),
                ]));
            }
        } else {
            lines.push(Line::styled(
                " No acceptance criteria defined",
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Findings for this task
        let task_findings: Vec<_> = app
            .findings
            .iter()
            .filter(|f| f.related_tasks.contains(&selected.task.id))
            .collect();

        if !task_findings.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                format!(" Findings ({}):", task_findings.len()),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));

            for finding in task_findings {
                let sev_color = match finding.severity {
                    crate::core::finding::FindingSeverity::Critical => Color::Red,
                    crate::core::finding::FindingSeverity::High => Color::LightRed,
                    crate::core::finding::FindingSeverity::Medium => Color::Yellow,
                    crate::core::finding::FindingSeverity::Low => Color::DarkGray,
                    crate::core::finding::FindingSeverity::Positive => Color::Green,
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", finding.id), Style::default().fg(Color::Cyan)),
                    Span::raw(truncate_chars(&finding.description, 40)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("severity: {}", finding.severity),
                        Style::default().fg(sev_color),
                    ),
                ]));
            }
        }
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(" Criteria & Findings ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White)),
    );

    f.render_widget(paragraph, area);
}

fn render_input(f: &mut Frame, app: &UatApp, area: Rect) {
    let (style, text) = if app.input_active {
        (
            Style::default().fg(Color::Cyan),
            format!(" > {}\u{2588}", app.input_buffer),
        )
    } else if let Some(msg) = &app.status_message {
        (Style::default().fg(Color::Green), format!("  {msg}"))
    } else {
        (
            Style::default().fg(Color::DarkGray),
            "  Press Enter or 'f' to capture a finding".to_string(),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .style(style);
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, app: &UatApp, area: Rect) {
    let (total, passed, with_findings, untested) = app.stats();

    let footer = Line::from(vec![
        Span::styled(" \u{2191}\u{2193} ", Style::default().fg(Color::Yellow)),
        Span::raw("navigate "),
        Span::styled(" Enter/f ", Style::default().fg(Color::Yellow)),
        Span::raw("capture "),
        Span::styled(" p ", Style::default().fg(Color::Yellow)),
        Span::raw("pass "),
        Span::styled(" q ", Style::default().fg(Color::Yellow)),
        Span::raw("quit "),
        Span::raw(" | "),
        Span::styled(format!("{passed}"), Style::default().fg(Color::Green)),
        Span::raw(" pass "),
        Span::styled(format!("{with_findings}"), Style::default().fg(Color::Red)),
        Span::raw(" issues "),
        Span::styled(format!("{untested}"), Style::default().fg(Color::DarkGray)),
        Span::raw(format!(" untested / {total}")),
    ]);

    f.render_widget(Paragraph::new(footer), area);
}

/// UTF-8 safe truncation (never slice byte boundaries)
fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
```

---

## Part 4: Wire it up

### `src/tui/mod.rs`

Add the new modules:

```rust
pub mod app;
pub mod event;
pub mod ui;
pub mod uat_app;
pub mod uat_ui;
```

---

## Part 5: Tests

Add tests in `src/tui/uat_app.rs` (inside a `#[cfg(test)] mod tests` block):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_finding(id: &str, severity: crate::core::finding::FindingSeverity, related: Vec<String>) -> Finding {
        Finding {
            id: id.to_string(),
            description: format!("Finding {id}"),
            severity,
            finding_type: crate::core::finding::FindingType::Bug,
            related_tasks: related,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_uat_status_initial_untested() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        // Create a completed build task
        let task = Task::new("T-001", "Fix audio loop", "desc");
        let mut task = task;
        task.status = TaskStatus::Completed;
        task.phase = Some(TaskPhase::Build);
        task.acceptance_criteria = vec!["Audio works".into()];
        let task_mgr = TaskManager::new(&forge_dir);
        task_mgr.save_task(&task).unwrap();

        let app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].uat_status, UatStatus::Untested);
    }

    #[test]
    fn test_uat_filters_verify_subtasks() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);

        // Build task (should appear)
        let mut t1 = Task::new("T-001", "Build thing", "desc");
        t1.status = TaskStatus::Completed;
        t1.phase = Some(TaskPhase::Build);
        task_mgr.save_task(&t1).unwrap();

        // Verify subtask (should be filtered out)
        let mut v1 = Task::new("V-001", "Verify thing", "desc");
        v1.status = TaskStatus::Completed;
        v1.phase = Some(TaskPhase::Verify);
        task_mgr.save_task(&v1).unwrap();

        let app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].task.id, "T-001");
    }

    #[test]
    fn test_uat_pass_marks_task() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let mut t = Task::new("T-001", "Test", "desc");
        t.status = TaskStatus::Completed;
        let task_mgr = TaskManager::new(&forge_dir);
        task_mgr.save_task(&t).unwrap();

        let mut app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        assert_eq!(app.tasks[0].uat_status, UatStatus::Untested);

        app.handle_key(KeyEvent::from(KeyCode::Char('p')));
        assert_eq!(app.tasks[0].uat_status, UatStatus::Passed);
    }

    #[test]
    fn test_uat_stats() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        for i in 1..=3 {
            let mut t = Task::new(&format!("T-{i:03}"), &format!("Task {i}"), "desc");
            t.status = TaskStatus::Completed;
            task_mgr.save_task(&t).unwrap();
        }

        let mut app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        assert_eq!(app.stats(), (3, 0, 0, 3)); // all untested

        app.handle_key(KeyEvent::from(KeyCode::Char('p')));
        assert_eq!(app.stats(), (3, 1, 0, 2)); // 1 passed
    }

    #[test]
    fn test_uat_capture_finding_updates_status() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let mut t = Task::new("T-001", "Audio system", "desc");
        t.status = TaskStatus::Completed;
        let task_mgr = TaskManager::new(&forge_dir);
        task_mgr.save_task(&t).unwrap();

        let mut app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();

        // Enter input mode
        app.handle_key(KeyEvent::from(KeyCode::Char('f')));
        assert!(app.input_active);

        // Type a finding
        for c in "audio echo broken".chars() {
            app.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }

        // Submit
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(!app.input_active);
        assert_eq!(app.tasks[0].uat_status, UatStatus::HasFindings);
        assert_eq!(app.tasks[0].finding_count, 1);
        assert_eq!(app.findings.len(), 1);
        assert_eq!(app.findings[0].id, "F-001");
    }

    #[test]
    fn test_uat_escape_cancels_input() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let mut t = Task::new("T-001", "Test", "desc");
        t.status = TaskStatus::Completed;
        TaskManager::new(&forge_dir).save_task(&t).unwrap();

        let mut app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        app.handle_key(KeyEvent::from(KeyCode::Char('f')));
        assert!(app.input_active);

        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(!app.input_active);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn test_uat_navigation() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        for i in 1..=3 {
            let mut t = Task::new(&format!("T-{i:03}"), &format!("Task {i}"), "desc");
            t.status = TaskStatus::Completed;
            task_mgr.save_task(&t).unwrap();
        }

        let mut app = UatApp::new(forge_dir, tmp.path().to_path_buf()).unwrap();
        assert_eq!(app.selected_task, 0);

        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_task, 1);

        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_task, 2);

        // Can't go past end
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_task, 2);

        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.selected_task, 1);
    }

    #[test]
    fn test_uat_inline_capture() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let mut t = Task::new("T-001", "Audio system", "desc");
        t.status = TaskStatus::Completed;
        TaskManager::new(&forge_dir).save_task(&t).unwrap();

        // Test inline capture (the CLI path)
        super::execute_inline(&forge_dir, "the audio crashes on submit").unwrap();

        let finding_mgr = FindingManager::new(&forge_dir);
        let findings = finding_mgr.list_findings().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "F-001");
    }

    #[test]
    fn test_truncate_chars_utf8_safe() {
        use crate::tui::uat_ui::truncate_chars;
        // If truncate_chars is private, move this test to uat_ui or make it pub(crate)
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello world this is long", 10), "hello w...");
        // Multi-byte: em dash
        let s = "Fix \u{2014} audio feedback loop issue";
        let result = truncate_chars(&s, 15);
        assert!(result.chars().count() <= 15);
        assert!(result.ends_with("..."));
    }
}
```

**NOTE:** The `execute_inline` function needs to be `pub(crate)` (not private) so the test can call it. Also, `truncate_chars` in `uat_ui.rs` needs to be `pub(crate)` for the UTF-8 test (or move that test to `uat_ui.rs`).

---

## Files Summary

| File | Action | What |
|------|--------|------|
| `src/cli/mod.rs` | Modify | Add `finding: Option<String>` arg to `Uat` variant |
| `src/main.rs` | Modify | Pass `finding` arg to `uat::execute` |
| `src/cli/uat.rs` | Rewrite | Inline mode + TUI launch (replace old REPL) |
| `src/tui/mod.rs` | Modify | Add `pub mod uat_app; pub mod uat_ui;` |
| `src/tui/uat_app.rs` | New | UAT app state, key handling, finding capture, `run()` |
| `src/tui/uat_ui.rs` | New | UAT rendering (header, task list, criteria, input, footer) |

## IMPORTANT NOTES

- Do NOT modify `src/tui/app.rs` or `src/tui/ui.rs` — the dashboard is unchanged
- The UAT TUI is completely separate from the dashboard TUI
- Use the same ratatui + crossterm setup pattern as `src/cli/dashboard.rs`
- The `Task::new()` constructor exists and takes `(id, title, description)` — use it in tests
- `TaskManager::save_task()` exists — use it to create test fixtures
- `FindingManager` is NOT `Clone` — store it directly in `UatApp`
- UTF-8: ALWAYS use `.chars().count()` and `.chars().take(n)` — NEVER byte slice strings
- The `colored` crate is for CLI output only. Inside ratatui use `Style::default().fg(Color::X)`

---

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass.**
