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
        // Ctrl+C always quits (even when typing)
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
                self.status_message =
                    Some("Type finding, press Enter to capture, Esc to cancel".into());
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
        if let Some(selected) = self.tasks.get(self.selected_task)
            && !related.contains(&selected.task.id)
        {
            related.insert(0, selected.task.id.clone());
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
        let passed = self
            .tasks
            .iter()
            .filter(|t| t.uat_status == UatStatus::Passed)
            .count();
        let with_findings = self
            .tasks
            .iter()
            .filter(|t| t.uat_status == UatStatus::HasFindings)
            .count();
        let untested = self
            .tasks
            .iter()
            .filter(|t| t.uat_status == UatStatus::Untested)
            .count();
        (total, passed, with_findings, untested)
    }
}

/// Entry point: launch the UAT TUI
pub fn run(forge_dir: PathBuf, project_root: PathBuf) -> anyhow::Result<()> {
    use colored::Colorize;

    let mut app = UatApp::new(forge_dir, project_root)?;

    if app.tasks.is_empty() {
        println!("  {} No completed tasks to test.", "!".yellow());
        println!(
            "  Run the dashboard first to complete tasks, then run {} for UAT.",
            "forge uat".cyan()
        );
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

        if crossterm::event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key);
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
    let bugs = app
        .findings
        .iter()
        .filter(|f| !matches!(f.severity, crate::core::finding::FindingSeverity::Positive))
        .count();

    println!();
    println!(
        "  UAT Summary: {} total | {} passed | {} with findings | {} untested",
        total, passed, with_findings, untested
    );
    println!(
        "  {} findings captured ({} issues).",
        app.findings.len(),
        bugs
    );

    if bugs > 0 {
        println!(
            "  Run {} to generate fix tasks.",
            "forge plan --from-findings".cyan()
        );
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::uat::execute_inline;

    #[test]
    fn test_uat_status_initial_untested() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let mut task = Task::new("T-001", "Fix audio loop", "desc");
        task.status = TaskStatus::Completed;
        task.phase = Some(TaskPhase::Build);
        task.acceptance_criteria = vec!["Audio works".into()];
        let task_mgr = TaskManager::new(&forge_dir);
        task_mgr.create_task(&task).unwrap();

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
        task_mgr.create_task(&t1).unwrap();

        // Verify subtask (should be filtered out)
        let mut v1 = Task::new("V-001", "Verify thing", "desc");
        v1.status = TaskStatus::Completed;
        v1.phase = Some(TaskPhase::Verify);
        task_mgr.create_task(&v1).unwrap();

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
        task_mgr.create_task(&t).unwrap();

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
            task_mgr.create_task(&t).unwrap();
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
        task_mgr.create_task(&t).unwrap();

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
        TaskManager::new(&forge_dir).create_task(&t).unwrap();

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
            task_mgr.create_task(&t).unwrap();
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
        TaskManager::new(&forge_dir).create_task(&t).unwrap();

        // Test inline capture (the CLI path)
        execute_inline(&forge_dir, "the audio crashes on submit").unwrap();

        let finding_mgr = FindingManager::new(&forge_dir);
        let findings = finding_mgr.list_findings().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "F-001");
    }

    #[test]
    fn test_truncate_chars_utf8_safe() {
        use crate::tui::uat_ui::truncate_chars;
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello world this is long", 10), "hello w...");
        // Multi-byte: em dash
        let s = "Fix \u{2014} audio feedback loop issue";
        let result = truncate_chars(s, 15);
        assert!(result.chars().count() <= 15);
        assert!(result.ends_with("..."));
    }
}
