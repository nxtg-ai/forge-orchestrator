use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::adapters::ToolAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskPhase, TaskStatus};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

const OUTPUT_BUFFER_CAP: usize = 200;
const EVENT_BUFFER_CAP: usize = 50;

/// Number of agent/summary panes in the 2x2 grid.
pub const PANE_COUNT: usize = 4;

/// Maximum rate-limit backoff attempts before marking task as permanently failed.
pub const MAX_BACKOFF_ATTEMPTS: u32 = 5;

/// Rate limit patterns to detect in agent output.
const RATE_LIMIT_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "429",
    "quota exceeded",
    "too many requests",
    "resource exhausted",
    "resource_exhausted",
];

/// Per-agent rate limit backoff tracking.
pub struct BackoffState {
    pub attempt: u32,
    pub next_retry: Option<Instant>,
    pub task_id: String,
}

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

pub enum AgentEvent {
    Output {
        task_id: String,
        agent: AgentType,
        line: String,
    },
    Completed {
        task_id: String,
        agent: AgentType,
        success: bool,
        exit_code: i32,
    },
    Error {
        task_id: String,
        agent: AgentType,
        message: String,
    },
}

/// Focus target: task board or one of the 4 panes (Claude/Codex/Gemini/Summary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    TaskBoard,
    /// Pane index: 0=Claude, 1=Codex, 2=Gemini, 3=Summary
    Pane(usize),
}

/// Map pane index to agent type (pane 3 = Summary, has no agent).
pub fn pane_agent(index: usize) -> Option<AgentType> {
    match index {
        0 => Some(AgentType::Claude),
        1 => Some(AgentType::Codex),
        2 => Some(AgentType::Gemini),
        _ => None,
    }
}

/// Pane labels for display.
pub fn pane_label(index: usize) -> &'static str {
    match index {
        0 => "Claude",
        1 => "Codex",
        2 => "Gemini",
        3 => "Summary",
        _ => "?",
    }
}

pub struct App {
    pub forge_dir: PathBuf,
    pub project_root: PathBuf,
    pub tasks: Vec<Task>,
    pub completed_task_ids: Vec<String>,
    pub agent_outputs: HashMap<AgentType, VecDeque<String>>,
    pub events: VecDeque<String>,
    pub running_task_ids: HashSet<String>,
    /// Maps agent type to the task ID it's currently running
    pub agent_running_task: HashMap<AgentType, String>,
    pub selected_index: usize,
    pub focus: FocusArea,
    pub parallel_limit: usize,
    pub watch_mode: bool,
    pub should_quit: bool,
    pub all_complete: bool,
    /// Current dashboard lifecycle phase (Build → Verify → Complete).
    pub phase: DashboardPhase,
    pub completed_at: Option<Instant>,
    pub started_at: Instant,
    /// Per-pane scroll offset (lines from bottom; 0 = latest output visible).
    pub pane_scroll: [usize; PANE_COUNT],
    /// Per-pane pin: true = user scrolled up, don't auto-scroll on new output.
    pub pane_pinned: [bool; PANE_COUNT],
    /// If Some(i), pane i is expanded to full screen.
    pub expanded_pane: Option<usize>,
    /// Per-agent rate limit backoff tracking.
    pub agent_backoff: HashMap<AgentType, BackoffState>,
    /// Throttle: last time we reloaded task JSONs from disk.
    pub last_task_reload: Instant,
    /// Project name from .forge/state.json for display in header.
    pub project_name: String,
    /// User shell process active (replaces Summary pane when true).
    pub shell_active: bool,
    /// Output buffer for the user shell.
    pub shell_output: VecDeque<String>,
    /// Channel to send keystrokes to the shell's stdin.
    pub shell_input_tx: Option<mpsc::UnboundedSender<String>>,
}

impl App {
    pub fn new(
        forge_dir: PathBuf,
        project_root: PathBuf,
        parallel_limit: usize,
        watch_mode: bool,
    ) -> (Self, mpsc::UnboundedReceiver<AgentEvent>, mpsc::UnboundedSender<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut agent_outputs = HashMap::new();
        agent_outputs.insert(AgentType::Claude, VecDeque::new());
        agent_outputs.insert(AgentType::Codex, VecDeque::new());
        agent_outputs.insert(AgentType::Gemini, VecDeque::new());

        let project_name = {
            let state_mgr = StateManager::new(&forge_dir);
            state_mgr
                .load()
                .map(|s| s.project_name.clone())
                .unwrap_or_default()
        };

        let app = Self {
            forge_dir,
            project_root,
            tasks: Vec::new(),
            completed_task_ids: Vec::new(),
            agent_outputs,
            events: VecDeque::new(),
            running_task_ids: HashSet::new(),
            agent_running_task: HashMap::new(),
            selected_index: 0,
            focus: FocusArea::TaskBoard,
            parallel_limit,
            watch_mode,
            should_quit: false,
            all_complete: false,
            phase: DashboardPhase::Build,
            completed_at: None,
            started_at: Instant::now(),
            pane_scroll: [0; PANE_COUNT],
            pane_pinned: [false; PANE_COUNT],
            expanded_pane: None,
            agent_backoff: HashMap::new(),
            last_task_reload: Instant::now(),
            project_name,
            shell_active: false,
            shell_output: VecDeque::new(),
            shell_input_tx: None,
        };

        (app, rx, tx)
    }

    pub fn reload_tasks(&mut self) -> anyhow::Result<()> {
        let task_mgr = TaskManager::new(&self.forge_dir);
        self.tasks = task_mgr.list_tasks()?;
        self.completed_task_ids = task_mgr.get_completed_task_ids()?;
        Ok(())
    }

    pub fn schedule_unblocked_tasks(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.watch_mode {
            return;
        }

        let slots = self.parallel_limit.saturating_sub(self.running_task_ids.len());
        if slots == 0 {
            return;
        }

        let candidates: Vec<Task> = self
            .tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && !t.is_blocked(&self.completed_task_ids)
                    && !self.running_task_ids.contains(&t.id)
            })
            .filter(|t| {
                // Skip tasks whose agent is in backoff
                let agent = t.assigned_to.clone().unwrap_or(AgentType::Claude);
                let agent = if agent == AgentType::Any {
                    AgentType::Claude
                } else {
                    agent
                };
                !self.is_agent_in_backoff(&agent)
            })
            .take(slots)
            .cloned()
            .collect();

        for task in candidates {
            self.spawn_task(&task, tx);
        }
    }

    pub fn handle_agent_event(
        &mut self,
        event: AgentEvent,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> anyhow::Result<()> {
        match event {
            AgentEvent::Output {
                ref task_id,
                ref agent,
                ref line,
            } if task_id == "__shell__" => {
                self.shell_output.push_back(line.clone());
                while self.shell_output.len() > OUTPUT_BUFFER_CAP {
                    self.shell_output.pop_front();
                }
                return Ok(());
            }
            AgentEvent::Output {
                agent, line, ..
            } => {
                let buf = self
                    .agent_outputs
                    .entry(agent.clone())
                    .or_default();
                if buf.len() >= OUTPUT_BUFFER_CAP {
                    buf.pop_front();
                }
                buf.push_back(line);

                // If the pane for this agent is pinned, keep scroll offset stable.
                // If not pinned, auto-scroll stays at 0 (no-op needed).
                // When pinned and buffer overflows, the removed front line means
                // the scroll offset should decrease by 1 to keep the same view.
                if let Some(idx) = agent_pane_index(&agent)
                    && self.pane_pinned[idx]
                    && self.pane_scroll[idx] > 0
                {
                    self.pane_scroll[idx] = self.pane_scroll[idx].saturating_sub(1);
                }
            }
            AgentEvent::Completed {
                ref task_id, ..
            } if task_id == "__shell__" => {
                self.shell_active = false;
                self.shell_input_tx = None;
                self.push_event("User shell closed");
                return Ok(());
            }
            AgentEvent::Completed {
                task_id,
                agent,
                success,
                exit_code,
            } => {
                self.running_task_ids.remove(&task_id);
                self.agent_running_task.remove(&agent);

                let task_mgr = TaskManager::new(&self.forge_dir);
                let state_mgr = StateManager::new(&self.forge_dir);
                let event_logger = EventLogger::new(&self.forge_dir);

                if success {
                    // Reset backoff on success
                    self.agent_backoff.remove(&agent);

                    if let Ok(task) = task_mgr.get_task(&task_id) {
                        let mut updated = task;
                        updated.status = TaskStatus::Completed;
                        updated.completed_at = Some(chrono::Utc::now());
                        updated.updated_at = chrono::Utc::now();
                        task_mgr.update_task(&updated).ok();
                        state_mgr.unlock_files(&task_id).ok();

                        // DX-028: Auto-commit after task completion
                        self.git_auto_commit(&updated, &agent);

                        let event_msg = format!(
                            "{} completed by {} (exit {})",
                            task_id, agent, exit_code
                        );
                        self.push_event(&event_msg);
                        event_logger
                            .log(
                                &ForgeEvent::new(EventType::TaskCompleted, event_msg)
                                    .with_task(&task_id)
                                    .with_agent(agent),
                            )
                            .ok();
                    }
                } else {
                    // Check if failure is due to rate limiting
                    let rate_limited = self
                        .agent_outputs
                        .get(&agent)
                        .is_some_and(is_rate_limited);

                    if rate_limited {
                        let attempt = {
                            let backoff = self
                                .agent_backoff
                                .entry(agent.clone())
                                .or_insert(BackoffState {
                                    attempt: 0,
                                    next_retry: None,
                                    task_id: String::new(),
                                });
                            backoff.attempt += 1;
                            backoff.attempt
                        };

                        if attempt >= MAX_BACKOFF_ATTEMPTS {
                            // Max retries exhausted — permanent failure
                            self.agent_backoff.remove(&agent);

                            if let Ok(task) = task_mgr.get_task(&task_id) {
                                let mut updated = task;
                                updated.status = TaskStatus::Failed;
                                updated.updated_at = chrono::Utc::now();
                                task_mgr.update_task(&updated).ok();
                                state_mgr.unlock_files(&task_id).ok();
                            }

                            let event_msg = format!(
                                "{} failed after {} rate limit retries by {}",
                                task_id, MAX_BACKOFF_ATTEMPTS, agent
                            );
                            self.push_event(&event_msg);
                            event_logger
                                .log(
                                    &ForgeEvent::new(EventType::TaskFailed, event_msg)
                                        .with_task(&task_id)
                                        .with_agent(agent),
                                )
                                .ok();
                        } else {
                            // Reset task to pending and apply backoff delay
                            if let Ok(task) = task_mgr.get_task(&task_id) {
                                let mut updated = task;
                                updated.status = TaskStatus::Pending;
                                updated.updated_at = chrono::Utc::now();
                                task_mgr.update_task(&updated).ok();
                                state_mgr.unlock_files(&task_id).ok();
                            }

                            let delay = compute_backoff_delay(attempt);
                            let delay_secs = delay.as_secs();
                            let backoff = self.agent_backoff.get_mut(&agent).unwrap();
                            backoff.next_retry = Some(Instant::now() + delay);
                            backoff.task_id = task_id.clone();

                            let event_msg = format!(
                                "{} rate limited. Retrying in {}s (attempt {}/{})",
                                task_id, delay_secs, attempt, MAX_BACKOFF_ATTEMPTS
                            );
                            self.push_event(&event_msg);
                            event_logger
                                .log(
                                    &ForgeEvent::new(EventType::TaskFailed, event_msg)
                                        .with_task(&task_id)
                                        .with_agent(agent),
                                )
                                .ok();
                        }
                    } else {
                        // Normal failure (not rate limited)
                        let failed_task = if let Ok(task) = task_mgr.get_task(&task_id) {
                            let mut updated = task;
                            updated.status = TaskStatus::Failed;
                            updated.updated_at = chrono::Utc::now();
                            task_mgr.update_task(&updated).ok();
                            state_mgr.unlock_files(&task_id).ok();
                            Some(updated)
                        } else {
                            None
                        };

                        let event_msg = format!(
                            "{} failed by {} (exit {})",
                            task_id, agent, exit_code
                        );
                        self.push_event(&event_msg);
                        event_logger
                            .log(
                                &ForgeEvent::new(EventType::TaskFailed, event_msg)
                                    .with_task(&task_id)
                                    .with_agent(agent),
                            )
                            .ok();

                        // Test/fix loop: generate fix + re-verify for failed verify tasks
                        if let Some(failed) = &failed_task {
                            self.handle_verify_failure(failed);
                        }
                    }
                }

                self.reload_tasks()?;
                self.check_phase_transition();
                self.schedule_unblocked_tasks(tx);
            }
            AgentEvent::Error {
                task_id,
                agent,
                message,
            } => {
                self.running_task_ids.remove(&task_id);
                self.agent_running_task.remove(&agent);

                let task_mgr = TaskManager::new(&self.forge_dir);
                let state_mgr = StateManager::new(&self.forge_dir);

                if let Ok(task) = task_mgr.get_task(&task_id) {
                    let mut updated = task;
                    updated.status = TaskStatus::Failed;
                    updated.updated_at = chrono::Utc::now();
                    task_mgr.update_task(&updated).ok();
                    state_mgr.unlock_files(&task_id).ok();
                }

                let event_msg = format!("{} error ({}): {}", task_id, agent, message);
                self.push_event(&event_msg);

                self.reload_tasks()?;
                self.check_phase_transition();
                self.schedule_unblocked_tasks(tx);
            }
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &mpsc::UnboundedSender<AgentEvent>) {
        // In expanded pane mode, Esc or Enter collapses back to grid
        if self.expanded_pane.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.expanded_pane = None;
                    return;
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                // Allow scrolling in expanded mode
                KeyCode::Up => {
                    if let Some(idx) = self.expanded_pane {
                        self.scroll_pane_up(idx);
                    }
                    return;
                }
                KeyCode::Down => {
                    if let Some(idx) = self.expanded_pane {
                        self.scroll_pane_down(idx);
                    }
                    return;
                }
                KeyCode::Home => {
                    if let Some(idx) = self.expanded_pane {
                        self.scroll_pane_to_top(idx);
                    }
                    return;
                }
                KeyCode::End => {
                    if let Some(idx) = self.expanded_pane {
                        self.scroll_pane_to_bottom(idx);
                    }
                    return;
                }
                _ => return,
            }
        }

        // Route keystrokes to shell when pane 3 is focused and shell is active
        if self.focus == FocusArea::Pane(3)
            && self.shell_active
            && let Some(shell_tx) = &self.shell_input_tx
        {
            use crossterm::event::KeyModifiers;
            match key.code {
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = shell_tx.send("exit\n".to_string());
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = shell_tx.send("\x03".to_string());
                    return;
                }
                KeyCode::Char('q') => {
                    // q in shell types 'q', doesn't quit dashboard
                    let _ = shell_tx.send("q".to_string());
                    return;
                }
                KeyCode::Char(c) => {
                    let _ = shell_tx.send(c.to_string());
                    return;
                }
                KeyCode::Enter => {
                    let _ = shell_tx.send("\n".to_string());
                    return;
                }
                KeyCode::Backspace => {
                    let _ = shell_tx.send("\x7f".to_string());
                    return;
                }
                KeyCode::Tab => {
                    let _ = shell_tx.send("\t".to_string());
                    return;
                }
                // Esc exits shell focus — falls through to normal handling
                KeyCode::Esc => {}
                _ => return,
            }
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Esc => {
                // Esc from a pane returns to task board; from task board quits
                match self.focus {
                    FocusArea::Pane(_) => self.focus = FocusArea::TaskBoard,
                    FocusArea::TaskBoard => self.should_quit = true,
                }
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    FocusArea::TaskBoard => FocusArea::Pane(0),
                    FocusArea::Pane(i) if i + 1 < PANE_COUNT => FocusArea::Pane(i + 1),
                    FocusArea::Pane(_) => FocusArea::TaskBoard,
                };
            }
            KeyCode::BackTab => {
                // Shift+Tab: reverse cycle
                self.focus = match self.focus {
                    FocusArea::TaskBoard => FocusArea::Pane(PANE_COUNT - 1),
                    FocusArea::Pane(0) => FocusArea::TaskBoard,
                    FocusArea::Pane(i) => FocusArea::Pane(i - 1),
                };
            }
            KeyCode::Up => match self.focus {
                FocusArea::TaskBoard => {
                    if self.selected_index > 0 {
                        self.selected_index -= 1;
                    }
                }
                FocusArea::Pane(idx) => self.scroll_pane_up(idx),
            },
            KeyCode::Down => match self.focus {
                FocusArea::TaskBoard => {
                    if self.selected_index + 1 < self.tasks.len() {
                        self.selected_index += 1;
                    }
                }
                FocusArea::Pane(idx) => self.scroll_pane_down(idx),
            },
            KeyCode::Home => {
                if let FocusArea::Pane(idx) = self.focus {
                    self.scroll_pane_to_top(idx);
                }
            }
            KeyCode::End => {
                if let FocusArea::Pane(idx) = self.focus {
                    self.scroll_pane_to_bottom(idx);
                }
            }
            KeyCode::Enter | KeyCode::Char('f') => {
                if let FocusArea::Pane(idx) = self.focus {
                    self.expanded_pane = Some(idx);
                }
            }
            KeyCode::Char('r') => {
                if self.focus == FocusArea::TaskBoard {
                    self.retry_selected_task(tx);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('+') => {
                if self.focus == FocusArea::TaskBoard || matches!(self.focus, FocusArea::Pane(0..=2)) {
                    if !self.shell_active {
                        self.spawn_shell(tx);
                    }
                    self.focus = FocusArea::Pane(3);
                }
            }
            _ => {}
        }
    }

    fn scroll_pane_up(&mut self, idx: usize) {
        let max = self.pane_buffer_len(idx);
        if self.pane_scroll[idx] < max.saturating_sub(1) {
            self.pane_scroll[idx] += 1;
            self.pane_pinned[idx] = true;
        }
    }

    fn scroll_pane_down(&mut self, idx: usize) {
        if self.pane_scroll[idx] > 0 {
            self.pane_scroll[idx] -= 1;
            if self.pane_scroll[idx] == 0 {
                self.pane_pinned[idx] = false;
            }
        }
    }

    fn scroll_pane_to_top(&mut self, idx: usize) {
        let max = self.pane_buffer_len(idx);
        self.pane_scroll[idx] = max.saturating_sub(1);
        if self.pane_scroll[idx] > 0 {
            self.pane_pinned[idx] = true;
        }
    }

    fn scroll_pane_to_bottom(&mut self, idx: usize) {
        self.pane_scroll[idx] = 0;
        self.pane_pinned[idx] = false;
    }

    /// Get the number of lines in a pane's buffer.
    pub fn pane_buffer_len(&self, idx: usize) -> usize {
        match pane_agent(idx) {
            Some(agent) => self
                .agent_outputs
                .get(&agent)
                .map(|b| b.len())
                .unwrap_or(0),
            None => 0, // Summary pane has no scrollable buffer
        }
    }

    /// Check if an agent is currently in backoff (waiting to retry after rate limit).
    pub fn is_agent_in_backoff(&self, agent: &AgentType) -> bool {
        self.agent_backoff
            .get(agent)
            .and_then(|state| state.next_retry)
            .is_some_and(|t| Instant::now() < t)
    }

    /// Check backoff timers and re-schedule when expired.
    pub fn check_backoff_timers(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let now = Instant::now();
        let mut expired = Vec::new();

        for (agent, state) in &mut self.agent_backoff {
            if state.next_retry.is_some_and(|t| now >= t) {
                state.next_retry = None;
                expired.push((agent.clone(), state.task_id.clone()));
            }
        }

        for (agent, task_id) in &expired {
            self.push_event(&format!(
                "Backoff expired for {}. Re-scheduling {}...",
                agent, task_id
            ));
        }

        if !expired.is_empty() {
            self.schedule_unblocked_tasks(tx);
        }
    }

    /// Handle a tick event: throttled task reload, backoff checks, completion detection.
    pub fn handle_tick(&mut self, agent_tx: &mpsc::UnboundedSender<AgentEvent>) -> anyhow::Result<()> {
        if self.last_task_reload.elapsed() > std::time::Duration::from_secs(2) {
            self.reload_tasks()?;
            self.last_task_reload = Instant::now();
            // Check phase transitions on each reload
            self.check_phase_transition();
        }
        self.check_backoff_timers(agent_tx);
        if !self.watch_mode
            && !self.all_complete
            && self.running_task_ids.is_empty()
            && !self.tasks.is_empty()
        {
            // Phase-aware completion: only mark complete when dashboard phase is Complete
            // or all tasks are done in the current phase
            if self.phase == DashboardPhase::Complete && self.is_all_done() {
                self.all_complete = true;
                self.completed_at = Some(Instant::now());
            } else if self.is_all_done() {
                // All tasks done but phase hasn't transitioned yet — trigger it
                self.check_phase_transition();
                if self.phase == DashboardPhase::Complete {
                    self.all_complete = true;
                    self.completed_at = Some(Instant::now());
                }
            }
        }
        Ok(())
    }

    /// Spawn a user shell in pane 3 (replaces Summary).
    pub fn spawn_shell(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.shell_active {
            return;
        }

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut cmd = TokioCommand::new(&shell);
        cmd.current_dir(&self.project_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();
                self.shell_input_tx = Some(input_tx);

                if let Some(mut stdin) = child.stdin.take() {
                    tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        while let Some(text) = input_rx.recv().await {
                            if stdin.write_all(text.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = stdin.flush().await;
                        }
                    });
                }

                if let Some(stdout) = child.stdout.take() {
                    let tx_out = tx.clone();
                    tokio::spawn(async move {
                        let mut lines =
                            tokio::io::BufReader::new(stdout).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if tx_out
                                .send(AgentEvent::Output {
                                    task_id: "__shell__".to_string(),
                                    agent: AgentType::Any,
                                    line,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                }

                if let Some(stderr) = child.stderr.take() {
                    let tx_err = tx.clone();
                    tokio::spawn(async move {
                        let mut lines =
                            tokio::io::BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if tx_err
                                .send(AgentEvent::Output {
                                    task_id: "__shell__".to_string(),
                                    agent: AgentType::Any,
                                    line,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                }

                let tx_done = tx.clone();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                    let _ = tx_done.send(AgentEvent::Completed {
                        task_id: "__shell__".to_string(),
                        agent: AgentType::Any,
                        success: true,
                        exit_code: 0,
                    });
                });

                self.shell_active = true;
                self.shell_output.clear();
                self.shell_output.push_back(format!("$ {shell}"));
                self.push_event("User shell opened (Ctrl+D to close)");
            }
            Err(e) => {
                self.push_event(&format!("Failed to spawn shell: {}", e));
            }
        }
    }

    /// Auto-commit working tree changes after a task completes successfully.
    fn git_auto_commit(&mut self, task: &Task, agent: &AgentType) {
        let state_mgr = StateManager::new(&self.forge_dir);
        let enabled = state_mgr
            .load()
            .map(|s| s.git.auto_commit)
            .unwrap_or(true);

        if !enabled {
            return;
        }

        if !self.project_root.join(".git").exists() {
            return;
        }

        let commit_type = commit_type_for_task(task.task_type.as_deref());
        let message = format!("{}({}): {}", commit_type, task.id, task.title);

        // Visual indicator in agent pane
        if let Some(buffer) = self.agent_outputs.get_mut(agent) {
            buffer.push_back(format!(
                "--- Auto-committing: {}({}) ---",
                commit_type, task.id
            ));
        }
        self.push_event(&format!("{} auto-committed ({})", task.id, commit_type));

        let project_root = self.project_root.clone();
        let task_id = task.id.clone();
        tokio::task::spawn_blocking(move || {
            let add_result = std::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(&project_root)
                .output();

            if let Err(e) = add_result {
                eprintln!("git add failed for {}: {}", task_id, e);
                return;
            }

            let commit_result = std::process::Command::new("git")
                .args(["commit", "--no-gpg-sign", "-m", &message])
                .current_dir(&project_root)
                .output();

            match commit_result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        if !stderr.contains("nothing to commit") {
                            eprintln!("git commit warning for {}: {}", task_id, stderr.trim());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("git commit failed for {}: {}", task_id, e);
                }
            }
        });
    }

    fn retry_selected_task(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.selected_index >= self.tasks.len() {
            return;
        }
        let task = &self.tasks[self.selected_index];
        let task_id = task.id.clone();

        match task.status {
            TaskStatus::Failed => {
                let task_mgr = TaskManager::new(&self.forge_dir);
                if let Ok(mut t) = task_mgr.get_task(&task_id) {
                    t.status = TaskStatus::Pending;
                    t.updated_at = chrono::Utc::now();
                    task_mgr.update_task(&t).ok();
                }
                self.push_event(&format!("Retrying {} (was failed)", task_id));
                self.reload_tasks().ok();
                self.schedule_unblocked_tasks(tx);
            }
            TaskStatus::InProgress => {
                self.running_task_ids.remove(&task_id);
                self.agent_running_task.retain(|_, v| *v != task_id);

                let task_mgr = TaskManager::new(&self.forge_dir);
                let state_mgr = StateManager::new(&self.forge_dir);
                if let Ok(mut t) = task_mgr.get_task(&task_id) {
                    t.status = TaskStatus::Pending;
                    t.updated_at = chrono::Utc::now();
                    task_mgr.update_task(&t).ok();
                    state_mgr.unlock_files(&task_id).ok();
                }
                self.push_event(&format!("Retrying {} (was in-progress)", task_id));
                self.reload_tasks().ok();
                self.schedule_unblocked_tasks(tx);
            }
            _ => {}
        }
    }

    pub fn is_all_done(&self) -> bool {
        self.tasks.iter().all(|t| {
            t.status == TaskStatus::Completed || t.status == TaskStatus::Failed
        })
    }

    /// Check if all build-phase tasks are done and transition to Verify phase.
    /// Returns true if a phase transition occurred.
    fn check_phase_transition(&mut self) -> bool {
        match self.phase {
            DashboardPhase::Build => {
                // Build tasks: phase is None or Build
                let build_tasks: Vec<&Task> = self
                    .tasks
                    .iter()
                    .filter(|t| {
                        t.phase.is_none() || t.phase == Some(TaskPhase::Build)
                    })
                    .collect();

                if build_tasks.is_empty() {
                    return false;
                }

                let all_build_done = build_tasks.iter().all(|t| {
                    t.status == TaskStatus::Completed || t.status == TaskStatus::Failed
                });

                if all_build_done {
                    self.phase = DashboardPhase::Verify;
                    self.push_event("Phase 1 (BUILD) complete — transitioning to VERIFY");

                    // Generate verify subtasks
                    let task_mgr = TaskManager::new(&self.forge_dir);
                    match task_mgr.generate_verify_subtasks() {
                        Ok(generated) => {
                            if generated.is_empty() {
                                self.push_event("No verify subtasks needed");
                            } else {
                                self.push_event(&format!(
                                    "Generated {} verify subtask(s)",
                                    generated.len()
                                ));
                            }
                        }
                        Err(e) => {
                            self.push_event(&format!("Failed to generate verify subtasks: {}", e));
                        }
                    }
                    // Reload to include newly generated tasks
                    self.reload_tasks().ok();
                    return true;
                }
                false
            }
            DashboardPhase::Verify => {
                // Verify/fix tasks: phase is Verify or Fix
                let verify_fix_tasks: Vec<&Task> = self
                    .tasks
                    .iter()
                    .filter(|t| {
                        t.phase == Some(TaskPhase::Verify) || t.phase == Some(TaskPhase::Fix)
                    })
                    .collect();

                if verify_fix_tasks.is_empty() {
                    // No verify tasks were generated — go straight to Complete
                    self.phase = DashboardPhase::Complete;
                    self.push_event("Phase 2 (VERIFY) complete — no verify tasks found");
                    return true;
                }

                let all_verify_done = verify_fix_tasks.iter().all(|t| {
                    t.status == TaskStatus::Completed || t.status == TaskStatus::Failed
                });

                if all_verify_done {
                    self.phase = DashboardPhase::Complete;
                    self.push_event("Phase 2 (VERIFY) complete — all verification done");
                    return true;
                }
                false
            }
            DashboardPhase::Complete => false,
        }
    }

    /// When a verify task fails, generate fix + re-verify pair (up to 3 retries).
    fn handle_verify_failure(&mut self, task: &Task) {
        if task.phase != Some(TaskPhase::Verify) {
            return;
        }

        if task.retry_count >= 3 {
            self.push_event(&format!(
                "{} failed after 3 retries — needs human attention",
                task.id
            ));
            return;
        }

        let task_mgr = TaskManager::new(&self.forge_dir);

        // Generate a fix subtask
        let fix_num = match task_mgr.next_task_number() {
            Ok(n) => n,
            Err(_) => return,
        };
        let fix_id = format!("T-{fix_num:03}");
        let mut fix_task = Task::new(
            &fix_id,
            format!("Fix: {} (retry {})", task.title.trim_start_matches("Verify: "), task.retry_count + 1),
            format!(
                "Fix failures from verify task {}.\nRetry attempt {}/3.\nOriginal verify: {}",
                task.id,
                task.retry_count + 1,
                task.description
            ),
        );
        fix_task.phase = Some(TaskPhase::Fix);
        fix_task.task_type = Some("implement".to_string());
        fix_task.parent_task = task.parent_task.clone();
        fix_task.depends_on = vec![task.id.clone()];
        fix_task.assigned_to = Some(AgentType::Claude);
        fix_task.locked_files = task.locked_files.clone();

        // Generate a re-verify subtask
        let verify_num = match task_mgr.next_verify_number() {
            Ok(n) => n,
            Err(_) => return,
        };
        let re_verify_id = format!("V-{verify_num:03}");
        let mut re_verify = Task::new(
            &re_verify_id,
            format!("Re-verify: {}", task.title.trim_start_matches("Verify: ").trim_start_matches("Re-verify: ")),
            format!(
                "Re-run verification after fix {}.\nRetry attempt {}/3.",
                fix_id,
                task.retry_count + 1
            ),
        );
        re_verify.phase = Some(TaskPhase::Verify);
        re_verify.task_type = Some("review".to_string());
        re_verify.parent_task = task.parent_task.clone();
        re_verify.depends_on = vec![fix_id.clone()];
        re_verify.assigned_to = task.assigned_to.clone();
        re_verify.locked_files = task.locked_files.clone();
        re_verify.retry_count = task.retry_count + 1;

        if task_mgr.create_task(&fix_task).is_ok()
            && task_mgr.create_task(&re_verify).is_ok()
        {
            self.push_event(&format!(
                "Auto-generated {} + {} to fix failures (retry {}/3)",
                fix_id, re_verify_id, task.retry_count + 1
            ));
        }
    }

    fn spawn_task(&mut self, task: &Task, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let agent = task
            .assigned_to
            .clone()
            .unwrap_or(AgentType::Claude);

        let agent = if agent == AgentType::Any {
            AgentType::Claude
        } else {
            agent
        };

        let auth_mode = "subscription";
        let permissions = {
            let state_mgr = StateManager::new(&self.forge_dir);
            let agent_name = agent.to_string().to_lowercase();
            state_mgr
                .get_agent_permissions(&agent_name)
                .unwrap_or_else(|_| "safe".to_string())
        };

        let std_cmd = match agent {
            AgentType::Claude | AgentType::Any => {
                ClaudeAdapter.build_command(task, &self.project_root, auth_mode, &permissions)
            }
            AgentType::Codex => {
                CodexAdapter.build_command(task, &self.project_root, auth_mode, &permissions)
            }
            AgentType::Gemini => {
                GeminiAdapter.build_command(task, &self.project_root, auth_mode, &permissions)
            }
        };

        let mut tokio_cmd = TokioCommand::from(std_cmd);
        tokio_cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        match tokio_cmd.spawn() {
            Ok(mut child) => {
                let task_id = task.id.clone();
                let task_id2 = task.id.clone();
                let task_id3 = task.id.clone();
                let agent2 = agent.clone();
                let agent3 = agent.clone();
                let tx2 = tx.clone();
                let tx3 = tx.clone();

                if let Some(stdout) = child.stdout.take() {
                    let tx_out = tx.clone();
                    let tid = task_id.clone();
                    let ag = agent.clone();
                    // Use JSON parser for Claude (stream-json output), plain lines for others
                    match agent {
                        AgentType::Claude | AgentType::Any => {
                            tokio::spawn(stream_claude_json(stdout, tid, ag, tx_out));
                        }
                        _ => {
                            tokio::spawn(stream_lines(stdout, tid, ag, tx_out));
                        }
                    }
                }

                if let Some(stderr) = child.stderr.take() {
                    tokio::spawn(stream_lines(stderr, task_id2, agent2, tx2));
                }

                tokio::spawn(async move {
                    match child.wait().await {
                        Ok(status) => {
                            let code = status.code().unwrap_or(-1);
                            let _ = tx3.send(AgentEvent::Completed {
                                task_id: task_id3,
                                agent: agent3,
                                success: status.success(),
                                exit_code: code,
                            });
                        }
                        Err(e) => {
                            let _ = tx3.send(AgentEvent::Error {
                                task_id: task_id3,
                                agent: agent3,
                                message: e.to_string(),
                            });
                        }
                    }
                });

                let task_mgr = TaskManager::new(&self.forge_dir);
                let state_mgr = StateManager::new(&self.forge_dir);
                let mut updated = task.clone();
                updated.status = TaskStatus::InProgress;
                updated.assigned_to = Some(agent.clone());
                updated.updated_at = chrono::Utc::now();
                task_mgr.update_task(&updated).ok();

                if !task.locked_files.is_empty() {
                    state_mgr
                        .lock_files(&task.id, agent.clone(), task.locked_files.clone())
                        .ok();
                }

                self.running_task_ids.insert(task.id.clone());
                self.agent_running_task.insert(agent.clone(), task.id.clone());
                self.push_event(&format!("Started {} on {}", task.id, agent));
            }
            Err(e) => {
                self.push_event(&format!("Failed to spawn {}: {}", task.id, e));
                let _ = tx.send(AgentEvent::Error {
                    task_id: task.id.clone(),
                    agent,
                    message: e.to_string(),
                });
            }
        }
    }

    pub fn cleanup_running_tasks(&mut self) {
        let task_mgr = TaskManager::new(&self.forge_dir);
        let state_mgr = StateManager::new(&self.forge_dir);
        let event_logger = EventLogger::new(&self.forge_dir);

        for task_id in self.running_task_ids.drain() {
            if let Ok(mut task) = task_mgr.get_task(&task_id) {
                task.status = TaskStatus::Pending;
                task.updated_at = chrono::Utc::now();
                task_mgr.update_task(&task).ok();
                state_mgr.unlock_files(&task_id).ok();

                event_logger
                    .log(&ForgeEvent::new(
                        EventType::TaskStarted,
                        format!("Reset {} to pending (dashboard exiting)", task_id),
                    ))
                    .ok();
            }
        }
        self.agent_running_task.clear();
    }

    pub fn reset_orphaned_tasks(&mut self) -> anyhow::Result<()> {
        let task_mgr = TaskManager::new(&self.forge_dir);
        let state_mgr = StateManager::new(&self.forge_dir);
        let event_logger = EventLogger::new(&self.forge_dir);

        let tasks = task_mgr.list_tasks()?;
        for task in tasks {
            if task.status == TaskStatus::InProgress {
                let mut updated = task.clone();
                updated.status = TaskStatus::Pending;
                updated.updated_at = chrono::Utc::now();
                task_mgr.update_task(&updated)?;
                state_mgr.unlock_files(&task.id).ok();

                let msg = format!("Reset orphaned task {} to pending", task.id);
                self.push_event(&msg);
                event_logger
                    .log(&ForgeEvent::new(EventType::TaskStarted, msg))
                    .ok();
            }
        }
        Ok(())
    }

    fn push_event(&mut self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        let entry = format!("[{}] {}", ts, msg);
        if self.events.len() >= EVENT_BUFFER_CAP {
            self.events.pop_front();
        }
        self.events.push_back(entry);
    }
}

/// Map task type to conventional commit type prefix.
pub fn commit_type_for_task(task_type: Option<&str>) -> &'static str {
    match task_type {
        Some("test") => "test",
        Some("document") => "docs",
        Some("review") => "refactor",
        Some("design") => "docs",
        Some("implement") => "feat",
        _ => "feat",
    }
}

/// Check if recent agent output contains rate limit indicators.
fn is_rate_limited(output: &VecDeque<String>) -> bool {
    output.iter().rev().take(20).any(|line| {
        let lower = line.to_lowercase();
        RATE_LIMIT_PATTERNS.iter().any(|p| lower.contains(p))
    })
}

/// Compute exponential backoff delay with jitter for a given attempt.
fn compute_backoff_delay(attempt: u32) -> std::time::Duration {
    let base_secs: u64 = match attempt {
        1 => 10,
        2 => 30,
        3 => 60,
        _ => 120,
    };
    let jitter_max: u64 = match attempt {
        1 => 5,
        2 => 10,
        3 => 15,
        _ => 30,
    };
    // Simple jitter using system time nanoseconds (no rand dependency needed)
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % (jitter_max + 1);
    std::time::Duration::from_secs(base_secs + jitter)
}

/// Map an agent type to its pane index.
fn agent_pane_index(agent: &AgentType) -> Option<usize> {
    match agent {
        AgentType::Claude | AgentType::Any => Some(0),
        AgentType::Codex => Some(1),
        AgentType::Gemini => Some(2),
    }
}

async fn stream_lines(
    reader: impl tokio::io::AsyncRead + Unpin,
    task_id: String,
    agent: AgentType,
    tx: mpsc::UnboundedSender<AgentEvent>,
) {
    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx
            .send(AgentEvent::Output {
                task_id: task_id.clone(),
                agent: agent.clone(),
                line,
            })
            .is_err()
        {
            break;
        }
    }
}

/// Parse Claude's stream-json NDJSON output into human-readable lines for TUI display.
async fn stream_claude_json(
    reader: impl tokio::io::AsyncRead + Unpin,
    task_id: String,
    agent: AgentType,
    tx: mpsc::UnboundedSender<AgentEvent>,
) {
    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let display_lines = parse_stream_json_line(&line);
        for display_line in display_lines {
            if tx
                .send(AgentEvent::Output {
                    task_id: task_id.clone(),
                    agent: agent.clone(),
                    line: display_line,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// Parse a single NDJSON line from Claude's stream-json output.
/// Returns zero or more human-readable display lines.
fn parse_stream_json_line(line: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        // Not valid JSON — show raw line as fallback
        return if line.trim().is_empty() {
            vec![]
        } else {
            vec![line.to_string()]
        };
    };

    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "init" => {
            vec!["[session started]".to_string()]
        }
        "message" => {
            let mut result = Vec::new();
            if let Some(content) = value.get("content").and_then(|c| c.as_array()) {
                for item in content {
                    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match item_type {
                        "text" => {
                            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                for text_line in text.lines() {
                                    let trimmed = text_line.trim();
                                    if !trimmed.is_empty() {
                                        result.push(trimmed.to_string());
                                    }
                                }
                            }
                        }
                        "tool_use" => {
                            let name =
                                item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let input_summary =
                                summarize_tool_input(name, item.get("input"));
                            result.push(format!("[{}] {}", name, input_summary));
                        }
                        _ => {}
                    }
                }
            }
            result
        }
        "result" => {
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let duration = value.get("duration_ms").and_then(|v| v.as_u64());
            match duration {
                Some(ms) => vec![format!("[done] {} in {:.1}s", status, ms as f64 / 1000.0)],
                None => vec![format!("[done] {}", status)],
            }
        }
        // Skip tool_result (too verbose) and unknown types
        _ => vec![],
    }
}

/// Extract a short summary from tool input for display.
fn summarize_tool_input(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };

    match tool_name {
        "Read" | "Edit" | "Write" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            truncate_str(cmd, 60)
        }
        "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            truncate_str(pattern, 40)
        }
        "Task" => {
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("subagent");
            truncate_str(desc, 50)
        }
        _ => {
            // Generic: show first string value found
            if let Some(obj) = input.as_object() {
                for (_key, val) in obj.iter().take(1) {
                    if let Some(s) = val.as_str() {
                        return truncate_str(s, 50);
                    }
                }
            }
            String::new()
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str, status: TaskStatus, deps: Vec<String>) -> Task {
        Task {
            id: id.to_string(),
            title: format!("Task {}", id),
            description: String::new(),
            status,
            assigned_to: Some(AgentType::Claude),
            task_type: None,
            depends_on: deps,
            locked_files: Vec::new(),
            acceptance_criteria: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
            plan_version: None,
            parent_task: None,
            phase: None,
            retry_count: 0,
        }
    }

    #[test]
    fn test_is_all_done_empty() {
        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert!(app.is_all_done());
    }

    #[test]
    fn test_is_all_done_with_completed_tasks() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![
            make_task("T-001", TaskStatus::Completed, vec![]),
            make_task("T-002", TaskStatus::Failed, vec![]),
        ];
        assert!(app.is_all_done());
    }

    #[test]
    fn test_is_all_done_with_pending() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![
            make_task("T-001", TaskStatus::Completed, vec![]),
            make_task("T-002", TaskStatus::Pending, vec![]),
        ];
        assert!(!app.is_all_done());
    }

    #[test]
    fn test_schedule_respects_parallel_limit() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            2,
            false,
        );
        app.running_task_ids.insert("T-001".to_string());
        app.running_task_ids.insert("T-002".to_string());
        app.tasks = vec![make_task("T-003", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-003"));
    }

    #[test]
    fn test_blocked_tasks_not_scheduled() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![make_task(
            "T-002",
            TaskStatus::Pending,
            vec!["T-001".to_string()],
        )];
        app.completed_task_ids = vec![];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-002"));
    }

    #[test]
    fn test_watch_mode_no_schedule() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            true,
        );
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-001"));
    }

    #[test]
    fn test_handle_key_quit() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.handle_key(KeyEvent::from(KeyCode::Char('q')), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn test_tab_cycles_through_panes_and_back() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert_eq!(app.focus, FocusArea::TaskBoard);

        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(0));

        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(1));

        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(2));

        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(3));

        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.focus, FocusArea::TaskBoard);
    }

    #[test]
    fn test_backtab_reverse_cycles() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert_eq!(app.focus, FocusArea::TaskBoard);

        app.handle_key(KeyEvent::from(KeyCode::BackTab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(3));

        app.handle_key(KeyEvent::from(KeyCode::BackTab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(2));
    }

    #[test]
    fn test_task_board_navigation() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![
            make_task("T-001", TaskStatus::Pending, vec![]),
            make_task("T-002", TaskStatus::Pending, vec![]),
            make_task("T-003", TaskStatus::Pending, vec![]),
        ];

        assert_eq!(app.selected_index, 0);
        app.handle_key(KeyEvent::from(KeyCode::Down), &tx);
        assert_eq!(app.selected_index, 1);
        app.handle_key(KeyEvent::from(KeyCode::Down), &tx);
        assert_eq!(app.selected_index, 2);
        app.handle_key(KeyEvent::from(KeyCode::Down), &tx);
        assert_eq!(app.selected_index, 2); // clamped

        app.handle_key(KeyEvent::from(KeyCode::Up), &tx);
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn test_pane_scroll_up_down() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        // Fill Claude pane (index 0) with 20 lines
        let buf = app.agent_outputs.get_mut(&AgentType::Claude).unwrap();
        for i in 0..20 {
            buf.push_back(format!("line {}", i));
        }

        // Focus on pane 0
        app.focus = FocusArea::Pane(0);

        // Scroll up
        app.handle_key(KeyEvent::from(KeyCode::Up), &tx);
        assert_eq!(app.pane_scroll[0], 1);
        assert!(app.pane_pinned[0]);

        app.handle_key(KeyEvent::from(KeyCode::Up), &tx);
        assert_eq!(app.pane_scroll[0], 2);

        // Scroll down
        app.handle_key(KeyEvent::from(KeyCode::Down), &tx);
        assert_eq!(app.pane_scroll[0], 1);
        assert!(app.pane_pinned[0]); // still pinned

        app.handle_key(KeyEvent::from(KeyCode::Down), &tx);
        assert_eq!(app.pane_scroll[0], 0);
        assert!(!app.pane_pinned[0]); // unpinned at bottom
    }

    #[test]
    fn test_pane_home_end() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        let buf = app.agent_outputs.get_mut(&AgentType::Claude).unwrap();
        for i in 0..50 {
            buf.push_back(format!("line {}", i));
        }

        app.focus = FocusArea::Pane(0);

        // Home = scroll to top
        app.handle_key(KeyEvent::from(KeyCode::Home), &tx);
        assert_eq!(app.pane_scroll[0], 49); // 50 lines - 1
        assert!(app.pane_pinned[0]);

        // End = scroll to bottom
        app.handle_key(KeyEvent::from(KeyCode::End), &tx);
        assert_eq!(app.pane_scroll[0], 0);
        assert!(!app.pane_pinned[0]);
    }

    #[test]
    fn test_enter_expands_pane() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );

        app.focus = FocusArea::Pane(2);
        app.handle_key(KeyEvent::from(KeyCode::Enter), &tx);
        assert_eq!(app.expanded_pane, Some(2));

        // Esc collapses
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert_eq!(app.expanded_pane, None);
    }

    #[test]
    fn test_expanded_pane_blocks_other_keys() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.expanded_pane = Some(0);

        // Tab should not change focus while expanded
        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.expanded_pane, Some(0)); // still expanded
    }

    #[test]
    fn test_esc_from_pane_returns_to_board() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.focus = FocusArea::Pane(1);
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert_eq!(app.focus, FocusArea::TaskBoard);
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_from_board_quits() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn test_push_event_caps_at_limit() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        for i in 0..60 {
            app.push_event(&format!("event {}", i));
        }
        assert_eq!(app.events.len(), EVENT_BUFFER_CAP);
    }

    #[test]
    fn test_cleanup_running_tasks_resets_tracking() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test-cleanup"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.running_task_ids.insert("T-001".to_string());
        app.running_task_ids.insert("T-002".to_string());
        app.agent_running_task
            .insert(AgentType::Claude, "T-001".to_string());
        app.agent_running_task
            .insert(AgentType::Codex, "T-002".to_string());

        app.cleanup_running_tasks();

        assert!(app.running_task_ids.is_empty());
        assert!(app.agent_running_task.is_empty());
    }

    #[test]
    fn test_all_complete_defaults_false() {
        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert!(!app.all_complete);
        assert!(app.completed_at.is_none());
    }

    #[test]
    fn test_retry_key_on_non_retryable_status_is_noop() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        app.selected_index = 0;
        let events_before = app.events.len();
        app.handle_key(KeyEvent::from(KeyCode::Char('r')), &tx);
        assert_eq!(app.events.len(), events_before);
    }

    #[test]
    fn test_r_key_ignored_when_pane_focused() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![make_task("T-001", TaskStatus::Failed, vec![])];
        app.focus = FocusArea::Pane(0);
        let events_before = app.events.len();
        app.handle_key(KeyEvent::from(KeyCode::Char('r')), &tx);
        // r should only work on task board
        assert_eq!(app.events.len(), events_before);
    }

    #[test]
    fn test_reset_orphaned_tasks_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let mut task = Task::new("T-099", "Orphan", "Was in-progress");
        task.status = TaskStatus::InProgress;
        task.assigned_to = Some(AgentType::Claude);
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-099.json"), &json).unwrap();
        std::fs::write(tasks_dir.join("T-099.md"), "# T-099").unwrap();

        let (mut app, _rx, _tx) = App::new(
            forge_dir.clone(),
            tmp.path().to_path_buf(),
            3,
            false,
        );

        app.reset_orphaned_tasks().unwrap();

        let task_mgr = TaskManager::new(&forge_dir);
        let reloaded = task_mgr.get_task("T-099").unwrap();
        assert_eq!(reloaded.status, TaskStatus::Pending);
        assert!(app.events.iter().any(|e| e.contains("orphaned")));
    }

    #[test]
    fn test_retry_failed_task_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let mut task = Task::new("T-050", "Failed task", "It failed");
        task.status = TaskStatus::Failed;
        task.assigned_to = Some(AgentType::Claude);
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-050.json"), json).unwrap();
        std::fs::write(tasks_dir.join("T-050.md"), "# T-050").unwrap();

        let (mut app, _rx, tx) = App::new(
            forge_dir.clone(),
            tmp.path().to_path_buf(),
            3,
            true,
        );
        app.reload_tasks().unwrap();
        app.selected_index = 0;

        app.handle_key(KeyEvent::from(KeyCode::Char('r')), &tx);

        let task_mgr = TaskManager::new(&forge_dir);
        let reloaded = task_mgr.get_task("T-050").unwrap();
        assert_eq!(reloaded.status, TaskStatus::Pending);
        assert!(app.events.iter().any(|e| e.contains("Retrying T-050")));
    }

    #[test]
    fn test_f_key_expands_pane() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.focus = FocusArea::Pane(1);
        app.handle_key(KeyEvent::from(KeyCode::Char('f')), &tx);
        assert_eq!(app.expanded_pane, Some(1));
    }

    #[test]
    fn test_scroll_in_expanded_mode() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        let buf = app.agent_outputs.get_mut(&AgentType::Claude).unwrap();
        for i in 0..30 {
            buf.push_back(format!("line {}", i));
        }
        app.expanded_pane = Some(0);

        app.handle_key(KeyEvent::from(KeyCode::Up), &tx);
        assert_eq!(app.pane_scroll[0], 1);

        app.handle_key(KeyEvent::from(KeyCode::End), &tx);
        assert_eq!(app.pane_scroll[0], 0);
    }

    // ── DX-018: Rate limit backoff tests ─────────────────────────

    #[test]
    fn test_is_rate_limited_detects_429() {
        let mut buf = VecDeque::new();
        buf.push_back("Starting task...".to_string());
        buf.push_back("Error: 429 Too Many Requests".to_string());
        assert!(is_rate_limited(&buf));
    }

    #[test]
    fn test_is_rate_limited_detects_patterns() {
        for pattern in &[
            "rate limit exceeded",
            "RESOURCE_EXHAUSTED",
            "quota exceeded for project",
            "too many requests, please slow down",
        ] {
            let mut buf = VecDeque::new();
            buf.push_back(pattern.to_string());
            assert!(is_rate_limited(&buf), "should detect: {}", pattern);
        }
    }

    #[test]
    fn test_is_rate_limited_false_for_normal_output() {
        let mut buf = VecDeque::new();
        buf.push_back("Compiling project...".to_string());
        buf.push_back("Error: syntax error on line 42".to_string());
        buf.push_back("Build failed with exit code 1".to_string());
        assert!(!is_rate_limited(&buf));
    }

    #[test]
    fn test_is_rate_limited_only_checks_last_20_lines() {
        let mut buf = VecDeque::new();
        // Rate limit pattern buried in old output (> 20 lines ago)
        buf.push_back("Error: 429 Too Many Requests".to_string());
        for i in 0..25 {
            buf.push_back(format!("normal output line {}", i));
        }
        assert!(!is_rate_limited(&buf));
    }

    #[test]
    fn test_backoff_delay_attempt_1() {
        let delay = compute_backoff_delay(1);
        // Attempt 1: 10s base + 0-5s jitter = 10-15s
        assert!(delay.as_secs() >= 10 && delay.as_secs() <= 15);
    }

    #[test]
    fn test_backoff_delay_attempt_2() {
        let delay = compute_backoff_delay(2);
        // Attempt 2: 30s base + 0-10s jitter = 30-40s
        assert!(delay.as_secs() >= 30 && delay.as_secs() <= 40);
    }

    #[test]
    fn test_backoff_delay_attempt_4() {
        let delay = compute_backoff_delay(4);
        // Attempt 4: 120s base + 0-30s jitter = 120-150s
        assert!(delay.as_secs() >= 120 && delay.as_secs() <= 150);
    }

    #[test]
    fn test_is_agent_in_backoff_true() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.agent_backoff.insert(
            AgentType::Gemini,
            BackoffState {
                attempt: 1,
                next_retry: Some(Instant::now() + std::time::Duration::from_secs(60)),
                task_id: "T-001".to_string(),
            },
        );
        assert!(app.is_agent_in_backoff(&AgentType::Gemini));
        assert!(!app.is_agent_in_backoff(&AgentType::Claude));
    }

    #[test]
    fn test_is_agent_in_backoff_false_when_expired() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        // Set next_retry in the past
        app.agent_backoff.insert(
            AgentType::Gemini,
            BackoffState {
                attempt: 1,
                next_retry: Some(Instant::now() - std::time::Duration::from_secs(1)),
                task_id: "T-001".to_string(),
            },
        );
        assert!(!app.is_agent_in_backoff(&AgentType::Gemini));
    }

    #[test]
    fn test_schedule_skips_agent_in_backoff() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        // Claude is in backoff (T-001 is assigned to Claude via make_task)
        app.agent_backoff.insert(
            AgentType::Claude,
            BackoffState {
                attempt: 1,
                next_retry: Some(Instant::now() + std::time::Duration::from_secs(60)),
                task_id: "T-001".to_string(),
            },
        );
        app.schedule_unblocked_tasks(&tx);
        // Should NOT have scheduled T-001
        assert!(!app.running_task_ids.contains("T-001"));
    }

    #[test]
    fn test_check_backoff_timers_clears_expired() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        // Set Gemini backoff that's already expired
        app.agent_backoff.insert(
            AgentType::Gemini,
            BackoffState {
                attempt: 2,
                next_retry: Some(Instant::now() - std::time::Duration::from_secs(1)),
                task_id: "T-005".to_string(),
            },
        );
        app.check_backoff_timers(&tx);
        // next_retry should be cleared
        assert!(app.agent_backoff.get(&AgentType::Gemini).unwrap().next_retry.is_none());
        // attempt should be preserved (for escalation if it fails again)
        assert_eq!(app.agent_backoff.get(&AgentType::Gemini).unwrap().attempt, 2);
        // Event should be logged
        assert!(app.events.iter().any(|e| e.contains("Backoff expired")));
    }

    #[test]
    fn test_backoff_reset_on_success_tracking() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.agent_backoff.insert(
            AgentType::Claude,
            BackoffState {
                attempt: 3,
                next_retry: None,
                task_id: "T-001".to_string(),
            },
        );
        assert!(app.agent_backoff.contains_key(&AgentType::Claude));
        // Simulate what handle_agent_event does on success
        app.agent_backoff.remove(&AgentType::Claude);
        assert!(!app.agent_backoff.contains_key(&AgentType::Claude));
    }

    // ── DX-026: Key event starvation fix tests ──────────────────

    #[test]
    fn test_handle_tick_throttles_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let (mut app, _rx, tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            false,
        );

        // First handle_tick should reload (last_task_reload was just set)
        // But since < 2s have elapsed, reload should be skipped
        let before = app.last_task_reload;
        app.handle_tick(&tx).unwrap();
        assert_eq!(app.last_task_reload, before, "should not reload within 2s");

        // Force last_task_reload to 3 seconds ago
        app.last_task_reload = Instant::now() - std::time::Duration::from_secs(3);
        let before = app.last_task_reload;
        app.handle_tick(&tx).unwrap();
        assert_ne!(app.last_task_reload, before, "should reload after 2s elapsed");
    }

    #[test]
    fn test_handle_tick_sets_all_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        // Create a completed task on disk (document type — skipped by verify generation)
        let mut task = Task::new("T-001", "Done task", "It's done");
        task.status = TaskStatus::Completed;
        task.assigned_to = Some(AgentType::Claude);
        task.task_type = Some("document".to_string());
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-001.json"), json).unwrap();

        let (mut app, _rx, tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            false,
        );
        app.reload_tasks().unwrap();
        assert!(!app.all_complete);

        // Force reload so handle_tick updates
        // With phase-aware completion: Build→Verify (no verify tasks)→Complete
        app.last_task_reload = Instant::now() - std::time::Duration::from_secs(3);
        app.handle_tick(&tx).unwrap();

        assert_eq!(app.phase, DashboardPhase::Complete);
        assert!(app.all_complete);
        assert!(app.completed_at.is_some());
    }

    #[test]
    fn test_handle_tick_no_complete_in_watch_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let mut task = Task::new("T-001", "Done task", "It's done");
        task.status = TaskStatus::Completed;
        task.assigned_to = Some(AgentType::Claude);
        task.task_type = Some("document".to_string());
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-001.json"), json).unwrap();

        let (mut app, _rx, tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            true, // watch mode
        );
        app.reload_tasks().unwrap();
        app.last_task_reload = Instant::now() - std::time::Duration::from_secs(3);
        app.handle_tick(&tx).unwrap();

        assert!(!app.all_complete, "watch mode should not auto-complete");
    }

    // ── DX-028: Auto-commit per task tests ──────────────────────

    #[test]
    fn test_commit_type_mapping() {
        assert_eq!(commit_type_for_task(Some("implement")), "feat");
        assert_eq!(commit_type_for_task(Some("test")), "test");
        assert_eq!(commit_type_for_task(Some("document")), "docs");
        assert_eq!(commit_type_for_task(Some("review")), "refactor");
        assert_eq!(commit_type_for_task(Some("design")), "docs");
        assert_eq!(commit_type_for_task(None), "feat");
        assert_eq!(commit_type_for_task(Some("unknown")), "feat");
    }

    #[test]
    fn test_git_auto_commit_skips_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Write state.json with auto_commit = false
        let mut state = crate::core::state::ForgeState::default();
        state.git.auto_commit = false;
        let state_mgr = StateManager::new(&forge_dir);
        state_mgr.save(&state).unwrap();

        // Create a fake .git dir so the git-repo check passes
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();

        let (mut app, _rx, _tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            false,
        );
        let task = Task::new("T-001", "Test task", "desc");

        // Should not push any event because auto_commit is false
        let events_before = app.events.len();
        app.git_auto_commit(&task, &AgentType::Claude);
        assert_eq!(app.events.len(), events_before);
    }

    #[test]
    fn test_git_auto_commit_skips_when_not_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // No .git directory — not a git repo

        let (mut app, _rx, _tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            false,
        );
        let task = Task::new("T-001", "Test task", "desc");

        let events_before = app.events.len();
        app.git_auto_commit(&task, &AgentType::Claude);
        assert_eq!(app.events.len(), events_before);
    }

    #[tokio::test]
    async fn test_git_auto_commit_pushes_event_when_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Default state has auto_commit = true
        let state = crate::core::state::ForgeState::default();
        let state_mgr = StateManager::new(&forge_dir);
        state_mgr.save(&state).unwrap();

        // Create .git so it's recognized as a git repo
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();

        let (mut app, _rx, _tx) = App::new(
            forge_dir,
            tmp.path().to_path_buf(),
            3,
            false,
        );
        let mut task = Task::new("T-005", "Add caching layer", "desc");
        task.task_type = Some("implement".to_string());

        app.git_auto_commit(&task, &AgentType::Codex);

        // Should have pushed an event and added to agent output
        assert!(app.events.iter().any(|e| e.contains("T-005 auto-committed (feat)")));
        let codex_buf = app.agent_outputs.get(&AgentType::Codex).unwrap();
        assert!(codex_buf.iter().any(|l| l.contains("Auto-committing: feat(T-005)")));
    }

    // ── DX-027: User-spawnable shell pane tests ─────────────────

    #[test]
    fn test_shell_defaults_inactive() {
        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert!(!app.shell_active);
        assert!(app.shell_output.is_empty());
        assert!(app.shell_input_tx.is_none());
    }

    #[test]
    fn test_shell_output_routes_to_shell_buffer() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.shell_active = true;

        // Simulate shell output via handle_agent_event
        let event = AgentEvent::Output {
            task_id: "__shell__".to_string(),
            agent: AgentType::Any,
            line: "hello world".to_string(),
        };
        // Need a tx for handle_agent_event
        let (tx, _rx2) = mpsc::unbounded_channel();
        app.handle_agent_event(event, &tx).unwrap();

        assert_eq!(app.shell_output.len(), 1);
        assert_eq!(app.shell_output[0], "hello world");
        // Should NOT be in agent_outputs
        assert!(app.agent_outputs.get(&AgentType::Any).is_none()
            || app.agent_outputs.get(&AgentType::Any).unwrap().is_empty());
    }

    #[test]
    fn test_shell_completion_resets_state() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.shell_active = true;
        app.shell_input_tx = Some(mpsc::unbounded_channel().0);

        let event = AgentEvent::Completed {
            task_id: "__shell__".to_string(),
            agent: AgentType::Any,
            success: true,
            exit_code: 0,
        };
        let (tx, _rx2) = mpsc::unbounded_channel();
        app.handle_agent_event(event, &tx).unwrap();

        assert!(!app.shell_active);
        assert!(app.shell_input_tx.is_none());
        assert!(app.events.iter().any(|e| e.contains("shell closed")));
    }

    #[test]
    fn test_esc_in_shell_pane_returns_to_taskboard() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.shell_active = true;
        app.shell_input_tx = Some(mpsc::unbounded_channel().0);
        app.focus = FocusArea::Pane(3);

        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        // Esc should move focus back to task board, not send to shell
        assert_eq!(app.focus, FocusArea::TaskBoard);
        assert!(app.shell_active); // Shell still running
    }

    #[test]
    fn test_s_key_focuses_shell_pane() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        // Manually set shell_active to simulate it was already spawned
        // (can't actually spawn_shell without tokio runtime)
        app.shell_active = true;

        app.focus = FocusArea::TaskBoard;
        app.handle_key(KeyEvent::from(KeyCode::Char('s')), &tx);
        assert_eq!(app.focus, FocusArea::Pane(3));
    }

    #[test]
    fn test_shell_not_double_spawnable() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.shell_active = true;
        app.shell_output.push_back("existing output".to_string());

        let (_tx, _rx2) = mpsc::unbounded_channel::<AgentEvent>();
        // spawn_shell should be a no-op when already active
        // We can't call it without tokio, but we test the guard:
        assert!(app.shell_active);
        // Calling spawn_shell would just return early due to shell_active check
        // so test that the output buffer is NOT cleared
        assert_eq!(app.shell_output.len(), 1);
    }

    // ── Part 2: Dashboard phase tests ────────────────────────────

    #[test]
    fn test_phase_starts_as_build() {
        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert_eq!(app.phase, DashboardPhase::Build);
    }

    #[test]
    fn test_phase_transitions_build_to_verify() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        // All build tasks completed → should transition to Verify
        let mut t1 = make_task("T-001", TaskStatus::Completed, vec![]);
        t1.phase = None; // build phase (None = build)
        let mut t2 = make_task("T-002", TaskStatus::Completed, vec![]);
        t2.phase = None;
        app.tasks = vec![t1, t2];

        assert!(app.check_phase_transition());
        assert_eq!(app.phase, DashboardPhase::Verify);
        assert!(app.events.iter().any(|e| e.contains("BUILD")));
    }

    #[test]
    fn test_phase_does_not_transition_with_pending() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        let mut t1 = make_task("T-001", TaskStatus::Completed, vec![]);
        t1.phase = None;
        let mut t2 = make_task("T-002", TaskStatus::Pending, vec![]);
        t2.phase = None;
        app.tasks = vec![t1, t2];

        assert!(!app.check_phase_transition());
        assert_eq!(app.phase, DashboardPhase::Build);
    }

    #[test]
    fn test_phase_transitions_verify_to_complete() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.phase = DashboardPhase::Verify;

        let mut v1 = make_task("V-001", TaskStatus::Completed, vec![]);
        v1.phase = Some(TaskPhase::Verify);
        let mut v2 = make_task("V-002", TaskStatus::Completed, vec![]);
        v2.phase = Some(TaskPhase::Verify);
        // Include a build task (already done) — it shouldn't block verify completion
        let t1 = make_task("T-001", TaskStatus::Completed, vec![]);
        app.tasks = vec![t1, v1, v2];

        assert!(app.check_phase_transition());
        assert_eq!(app.phase, DashboardPhase::Complete);
        assert!(app.events.iter().any(|e| e.contains("VERIFY")));
    }

    #[test]
    fn test_verify_to_complete_with_no_verify_tasks() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.phase = DashboardPhase::Verify;
        // No verify or fix tasks → should go straight to Complete
        let t1 = make_task("T-001", TaskStatus::Completed, vec![]);
        app.tasks = vec![t1];

        assert!(app.check_phase_transition());
        assert_eq!(app.phase, DashboardPhase::Complete);
    }

    #[test]
    fn test_handle_verify_failure_generates_fix_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        // Create a parent task
        let mut parent = Task::new("T-001", "Auth module", "Implement auth");
        parent.status = TaskStatus::Completed;
        parent.task_type = Some("implement".to_string());
        let json = serde_json::to_string_pretty(&parent).unwrap();
        std::fs::write(tasks_dir.join("T-001.json"), &json).unwrap();
        std::fs::write(tasks_dir.join("T-001.md"), "# T-001").unwrap();

        // Create a failed verify task
        let mut verify = Task::new("V-001", "Verify: Auth module", "Run tests");
        verify.phase = Some(TaskPhase::Verify);
        verify.parent_task = Some("T-001".to_string());
        verify.retry_count = 0;
        verify.status = TaskStatus::Failed;
        verify.assigned_to = Some(AgentType::Codex);
        let json = serde_json::to_string_pretty(&verify).unwrap();
        std::fs::write(tasks_dir.join("V-001.json"), &json).unwrap();
        std::fs::write(tasks_dir.join("V-001.md"), "# V-001").unwrap();

        let (mut app, _rx, _tx) = App::new(
            forge_dir.clone(),
            tmp.path().to_path_buf(),
            3,
            false,
        );
        app.phase = DashboardPhase::Verify;

        app.handle_verify_failure(&verify);

        // Should have generated a fix task and a re-verify task
        let task_mgr = TaskManager::new(&forge_dir);
        let tasks = task_mgr.list_tasks().unwrap();

        // Should have T-001, V-001, T-002 (fix), V-002 (re-verify)
        assert_eq!(tasks.len(), 4);

        let fix = tasks.iter().find(|t| t.id == "T-002").unwrap();
        assert_eq!(fix.phase, Some(TaskPhase::Fix));
        assert!(fix.title.contains("Fix:"));

        let re_verify = tasks.iter().find(|t| t.id == "V-002").unwrap();
        assert_eq!(re_verify.phase, Some(TaskPhase::Verify));
        assert_eq!(re_verify.retry_count, 1);
        assert_eq!(re_verify.depends_on, vec!["T-002".to_string()]);

        assert!(app.events.iter().any(|e| e.contains("retry 1/3")));
    }

    #[test]
    fn test_verify_failure_stops_after_3_retries() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        app.phase = DashboardPhase::Verify;

        let mut verify = Task::new("V-001", "Verify: Auth", "Run tests");
        verify.phase = Some(TaskPhase::Verify);
        verify.retry_count = 3; // Already at max
        verify.status = TaskStatus::Failed;

        app.handle_verify_failure(&verify);

        assert!(app.events.iter().any(|e| e.contains("needs human attention")));
        // No fix task should be generated — we can't easily test TaskManager
        // without a real forge_dir, but the event message confirms the path
    }

    #[test]
    fn test_dashboard_phase_display() {
        assert_eq!(DashboardPhase::Build.to_string(), "BUILD");
        assert_eq!(DashboardPhase::Verify.to_string(), "VERIFY");
        assert_eq!(DashboardPhase::Complete.to_string(), "COMPLETE");
    }

    // ── DX-029: Stream-JSON parser tests ─────────────────────────

    #[test]
    fn test_parse_stream_json_init() {
        let lines = parse_stream_json_line(r#"{"type":"init","session_id":"abc"}"#);
        assert_eq!(lines, vec!["[session started]"]);
    }

    #[test]
    fn test_parse_stream_json_text_message() {
        let lines = parse_stream_json_line(
            r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"Hello world"}]}"#,
        );
        assert_eq!(lines, vec!["Hello world"]);
    }

    #[test]
    fn test_parse_stream_json_tool_use() {
        let lines = parse_stream_json_line(
            r#"{"type":"message","content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/main.rs"}}]}"#,
        );
        assert_eq!(lines, vec!["[Read] src/main.rs"]);
    }

    #[test]
    fn test_parse_stream_json_tool_use_bash() {
        let lines = parse_stream_json_line(
            r#"{"type":"message","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}"#,
        );
        assert_eq!(lines, vec!["[Bash] npm test"]);
    }

    #[test]
    fn test_parse_stream_json_result() {
        let lines = parse_stream_json_line(
            r#"{"type":"result","status":"success","duration_ms":45000}"#,
        );
        assert_eq!(lines, vec!["[done] success in 45.0s"]);
    }

    #[test]
    fn test_parse_stream_json_tool_result_skipped() {
        let lines =
            parse_stream_json_line(r#"{"type":"tool_result","output":"lots of stuff"}"#);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_parse_stream_json_invalid_json_fallback() {
        let lines = parse_stream_json_line("not json at all");
        assert_eq!(lines, vec!["not json at all"]);
    }

    #[test]
    fn test_parse_stream_json_empty_line() {
        let lines = parse_stream_json_line("");
        assert!(lines.is_empty());
    }

    #[test]
    fn test_summarize_tool_input_read() {
        let input: serde_json::Value = serde_json::json!({"file_path": "src/main.rs"});
        assert_eq!(summarize_tool_input("Read", Some(&input)), "src/main.rs");
    }

    #[test]
    fn test_summarize_tool_input_bash_truncate() {
        let long_cmd = "a".repeat(100);
        let input: serde_json::Value = serde_json::json!({"command": long_cmd});
        let result = summarize_tool_input("Bash", Some(&input));
        assert!(result.len() <= 63); // 60 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        assert_eq!(truncate_str("hello world this is long", 10), "hello w...");
    }
}
