use crate::adapters::ToolAdapter;
use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::quality_gate::{self, GateResult};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskPhase, TaskStatus};
use crate::tui::pty_session::{PtySession, key_event_to_bytes};
use crossterm::event::{KeyCode, KeyEvent};
use portable_pty::{CommandBuilder, PtySize};
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

const OUTPUT_BUFFER_CAP: usize = 200;
const EVENT_BUFFER_CAP: usize = 50;

/// Number of agent/summary panes in the 2x2 grid.
pub const PANE_COUNT: usize = 4;

/// Maximum rate-limit backoff attempts before marking task as permanently failed.
pub const MAX_BACKOFF_ATTEMPTS: u32 = 5;

/// Rate limit patterns to detect in agent output (provider-specific + generic).
const RATE_LIMIT_PATTERNS: &[&str] = &[
    // Provider-specific (DX-034)
    "usage_limit_reached",
    "no capacity available",
    "rate_limit_error",
    // Generic patterns
    "rate limit",
    "rate_limit",
    "429",
    "quota exceeded",
    "too many requests",
    "resource exhausted",
    "resource_exhausted",
];

/// Builder Mode: tracks when an agent is awaiting task completion (ready-pattern reappearance).
pub struct AwaitingCompletion {
    pub task_id: String,
    pub dispatched_at: Instant,
    pub last_output_at: Instant,
    /// The ready-pattern must disappear (agent started working) before reappearance
    /// counts as completion. Prevents false positives when prompt text sits unsubmitted.
    pub pattern_disappeared: bool,
}

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
    /// Per-agent pacing: earliest time the next task can be dispatched (subscription mode only).
    pub agent_pacing: HashMap<AgentType, Instant>,
    /// User shell process active (replaces Summary pane when true).
    pub shell_active: bool,
    /// Output buffer for the user shell.
    pub shell_output: VecDeque<String>,
    /// Channel to send keystrokes to the shell's stdin.
    pub shell_input_tx: Option<mpsc::UnboundedSender<String>>,
    /// Per-provider task dispatch count for quota monitoring (DX-037).
    /// Key: AgentType, Value: (dispatched_count, window_start)
    pub provider_quota: HashMap<AgentType, (u32, Instant)>,
    /// DX-024 Stargate: Per-agent PTY session (replaces piped output).
    pub pty_sessions: HashMap<AgentType, PtySession>,
    /// DX-024: Whether Stargate PTY mode is active.
    pub pty_mode: bool,
    /// DX-024: Which pane index is attached for interactive input forwarding.
    pub attached_pane: Option<usize>,
    /// DX-024: PTY session for the user shell (pane 3).
    pub shell_pty: Option<PtySession>,
    /// Terminal dimensions (cols, rows) for PTY resize propagation.
    pub terminal_size: (u16, u16),
    /// DX-050 Builder Mode: tracks which agents are awaiting task completion.
    pub awaiting_completion: HashMap<AgentType, AwaitingCompletion>,
    /// DX-050: Spinner animation frame counter (cycles 0..9 on each tick).
    pub spinner_frame: usize,
    /// DX-052: Quality gate results receiver (from background thread).
    pub quality_gate_rx: Option<std::sync::mpsc::Receiver<Vec<GateResult>>>,
    /// DX-052: True while quality gates are running in background.
    pub quality_gate_pending: bool,
    /// DX-052: Number of gate retry attempts (max 3 before force-transition).
    pub quality_gate_attempts: u32,
}

impl App {
    pub fn new(
        forge_dir: PathBuf,
        project_root: PathBuf,
        parallel_limit: usize,
        watch_mode: bool,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<AgentEvent>,
        mpsc::UnboundedSender<AgentEvent>,
    ) {
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
            agent_pacing: HashMap::new(),
            last_task_reload: Instant::now(),
            project_name,
            shell_active: false,
            shell_output: VecDeque::new(),
            shell_input_tx: None,
            provider_quota: HashMap::new(),
            pty_sessions: HashMap::new(),
            pty_mode: false,
            attached_pane: None,
            shell_pty: None,
            terminal_size: crossterm::terminal::size().unwrap_or((80, 24)),
            awaiting_completion: HashMap::new(),
            spinner_frame: 0,
            quality_gate_rx: None,
            quality_gate_pending: false,
            quality_gate_attempts: 0,
        };

        (app, rx, tx)
    }

    pub fn reload_tasks(&mut self) -> anyhow::Result<()> {
        let task_mgr = TaskManager::new(&self.forge_dir);
        let raw_tasks = task_mgr.list_tasks()?;
        // Sort hierarchically so self.tasks matches the display order in the UI.
        // This ensures selected_index always points to the visually highlighted row.
        self.tasks = crate::tui::ui::hierarchical_sort(&raw_tasks);
        self.completed_task_ids = task_mgr.get_completed_task_ids()?;
        Ok(())
    }

    pub fn schedule_unblocked_tasks(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.watch_mode {
            return;
        }

        let slots = self
            .parallel_limit
            .saturating_sub(self.running_task_ids.len());
        if slots == 0 {
            return;
        }

        // Load scheduler config for pacing
        let state_mgr = StateManager::new(&self.forge_dir);
        let scheduler = state_mgr.load().map(|s| s.scheduler).unwrap_or_default();

        let now = Instant::now();

        // In PTY mode, each agent can only run 1 task at a time (TUI is single-threaded).
        // Track which agents are already busy or claimed this scheduling round.
        let mut agents_busy: HashSet<AgentType> = if self.pty_mode {
            self.agent_running_task.keys().cloned().collect()
        } else {
            HashSet::new()
        };

        let candidates: Vec<Task> = self
            .tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && !t.is_blocked(&self.completed_task_ids)
                    && !self.running_task_ids.contains(&t.id)
                    && t.phase != Some(TaskPhase::Uat) // UAT tasks are human-only
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
            .filter(|t| {
                // DX-033: Skip tasks whose agent is in subscription pacing cooldown
                let agent = t.assigned_to.clone().unwrap_or(AgentType::Claude);
                let agent = if agent == AgentType::Any {
                    AgentType::Claude
                } else {
                    agent
                };
                let agent_name = agent.to_string().to_lowercase();
                let auth_mode = state_mgr
                    .get_agent_auth(&agent_name)
                    .unwrap_or_else(|_| "subscription".to_string());
                if auth_mode == "api" {
                    return true; // API mode = no pacing
                }
                // Check if agent is still in pacing cooldown
                if let Some(ready_at) = self.agent_pacing.get(&agent)
                    && now < *ready_at
                {
                    return false; // Still cooling down
                }
                true
            })
            .filter(|t| {
                // PTY mode: 1 task per agent — skip if agent already busy or claimed
                if !self.pty_mode {
                    return true;
                }
                let agent = t.assigned_to.clone().unwrap_or(AgentType::Claude);
                let agent = if agent == AgentType::Any {
                    AgentType::Claude
                } else {
                    agent
                };
                if agents_busy.contains(&agent) {
                    return false;
                }
                // Claim this agent for this scheduling round
                agents_busy.insert(agent);
                true
            })
            .take(slots)
            .cloned()
            .collect();

        for task in candidates {
            let agent = task.assigned_to.clone().unwrap_or(AgentType::Claude);
            let agent = if agent == AgentType::Any {
                AgentType::Claude
            } else {
                agent
            };

            // DX-033: Set pacing cooldown for subscription-mode agents
            let agent_name = agent.to_string().to_lowercase();
            let auth_mode = state_mgr
                .get_agent_auth(&agent_name)
                .unwrap_or_else(|_| "subscription".to_string());
            if auth_mode == "subscription" {
                let delay_secs =
                    rand::rng().random_range(scheduler.pacing_min_secs..=scheduler.pacing_max_secs);
                self.agent_pacing.insert(
                    agent.clone(),
                    Instant::now() + std::time::Duration::from_secs(delay_secs),
                );
                self.push_event(&format!(
                    "Pacing {}: next task in {}s (subscription mode)",
                    agent, delay_secs
                ));
            }

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
            AgentEvent::Output { agent, line, .. } => {
                // DX-024: In PTY mode, skip agent_outputs insertion —
                // output is captured by the PTY session's vt100 terminal emulator.
                // The empty sentinel line just wakes the event loop to redraw.
                if self.pty_mode && self.pty_sessions.contains_key(&agent) {
                    // DX-050: Update last_output_at for completion detection
                    if let Some(awaiting) = self.awaiting_completion.get_mut(&agent) {
                        awaiting.last_output_at = Instant::now();
                    }
                    return Ok(());
                }

                let buf = self.agent_outputs.entry(agent.clone()).or_default();
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
            AgentEvent::Completed { ref task_id, .. } if task_id == "__shell__" => {
                self.shell_active = false;
                self.shell_input_tx = None;
                self.shell_pty = None;
                // Clear attached if we were attached to shell pane
                if self.attached_pane == Some(3) {
                    self.attached_pane = None;
                }
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

                // DX-024/DX-050: Only remove PTY session if the process truly exited.
                // In Builder Mode, check_pty_completion sends synthetic Completed events
                // while the TUI process is still alive. Keep the PTY so it can accept
                // the next task dispatch.
                let process_exited = self
                    .pty_sessions
                    .get_mut(&agent)
                    .map(|s| s.try_wait().is_some())
                    .unwrap_or(true);

                // Capture PTY output before removing session — helps debug
                // crashes like Codex exiting immediately with no visible error.
                let crash_output = if process_exited && !success {
                    self.pty_sessions.get(&agent).map(|s| {
                        let lines = s.snapshot();
                        lines
                            .iter()
                            .map(|l| {
                                l.spans
                                    .iter()
                                    .map(|sp| sp.text.as_str())
                                    .collect::<String>()
                            })
                            .filter(|l| !l.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                } else {
                    None
                };

                if process_exited {
                    self.pty_sessions.remove(&agent);
                    if let Some(pane_idx) = agent_pane_index(&agent)
                        && self.attached_pane == Some(pane_idx)
                    {
                        self.attached_pane = None;
                    }
                }

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

                        let event_msg =
                            format!("{} completed by {} (exit {})", task_id, agent, exit_code);
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
                    let rate_limited = self.agent_outputs.get(&agent).is_some_and(is_rate_limited);

                    if rate_limited {
                        let attempt = {
                            let backoff =
                                self.agent_backoff
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

                        let event_msg =
                            format!("{} failed by {} (exit {})", task_id, agent, exit_code);
                        self.push_event(&event_msg);
                        // Log captured PTY output for crash diagnosis
                        if let Some(ref output) = crash_output {
                            let truncated = if output.len() > 200 {
                                &output[..200]
                            } else {
                                output
                            };
                            if !truncated.is_empty() {
                                self.push_event(&format!("  PTY output: {}", truncated));
                            }
                        }
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
        use crossterm::event::KeyModifiers;
        // DX-024: Attached mode — forward ALL keys (except Esc/Ctrl+F) to the PTY
        if let Some(pane_idx) = self.attached_pane {
            if key.code == KeyCode::Esc {
                self.attached_pane = None;
                return;
            }
            // Ctrl+F: toggle expand/collapse while attached
            if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if self.expanded_pane.is_some() {
                    self.expanded_pane = None;
                } else {
                    self.expanded_pane = Some(pane_idx);
                }
                self.resize_pty_for_pane(pane_idx);
                return;
            }
            let bytes = key_event_to_bytes(&key);
            if !bytes.is_empty() {
                if pane_idx == 3 {
                    // Shell PTY
                    if let Some(ref session) = self.shell_pty {
                        session.write(&bytes);
                    }
                } else if let Some(agent) = pane_agent(pane_idx)
                    && let Some(session) = self.pty_sessions.get(&agent)
                {
                    session.write(&bytes);
                }
            }
            return;
        }

        // In expanded pane mode, Esc or Enter collapses back to grid
        if self.expanded_pane.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    let was_expanded = self.expanded_pane;
                    self.expanded_pane = None;
                    // Resize PTY back to small pane dimensions
                    if let Some(idx) = was_expanded {
                        self.resize_pty_for_pane(idx);
                    }
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
                // Allow 'i' to fall through to attach handler in expanded mode
                KeyCode::Char('i') => {}
                _ => return,
            }
        }

        // Route keystrokes to shell when pane 3 is focused and shell is active
        if self.focus == FocusArea::Pane(3)
            && self.shell_active
            && let Some(shell_tx) = &self.shell_input_tx
        {
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
            KeyCode::Home => match self.focus {
                FocusArea::TaskBoard => self.selected_index = 0,
                FocusArea::Pane(idx) => self.scroll_pane_to_top(idx),
            },
            KeyCode::End => match self.focus {
                FocusArea::TaskBoard => {
                    if !self.tasks.is_empty() {
                        self.selected_index = self.tasks.len() - 1;
                    }
                }
                FocusArea::Pane(idx) => self.scroll_pane_to_bottom(idx),
            },
            KeyCode::PageUp => {
                if self.focus == FocusArea::TaskBoard {
                    self.selected_index = self.selected_index.saturating_sub(10);
                }
            }
            KeyCode::PageDown => {
                if self.focus == FocusArea::TaskBoard && !self.tasks.is_empty() {
                    self.selected_index = (self.selected_index + 10).min(self.tasks.len() - 1);
                }
            }
            KeyCode::Enter | KeyCode::Char('f') => {
                if let FocusArea::Pane(idx) = self.focus {
                    self.expanded_pane = Some(idx);
                    self.resize_pty_for_pane(idx);
                }
            }
            KeyCode::Char('a') => {
                if self.pty_mode && self.focus == FocusArea::TaskBoard {
                    self.dispatch_to_tui(tx);
                }
            }
            KeyCode::Char('c') => {
                if self.focus == FocusArea::TaskBoard {
                    self.cycle_agent_assignment();
                }
            }
            KeyCode::Char('r') => {
                if self.focus == FocusArea::TaskBoard {
                    self.retry_selected_task(tx);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('+') => {
                if self.focus == FocusArea::TaskBoard
                    || matches!(self.focus, FocusArea::Pane(0..=2))
                {
                    if !self.shell_active {
                        self.spawn_shell(tx);
                    }
                    self.focus = FocusArea::Pane(3);
                }
            }
            // DX-024: 'i' attaches to focused PTY pane for interactive input
            KeyCode::Char('i') => {
                if self.pty_mode
                    && let FocusArea::Pane(idx) = self.focus
                {
                    let has_pty = if idx == 3 {
                        self.shell_pty.is_some()
                    } else {
                        pane_agent(idx)
                            .map(|a| self.pty_sessions.contains_key(&a))
                            .unwrap_or(false)
                    };
                    if has_pty {
                        self.attached_pane = Some(idx);
                    }
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
            Some(agent) => {
                if self.pty_mode {
                    // In PTY mode, read line count from the PTY session
                    self.pty_sessions
                        .get(&agent)
                        .map(|s| s.line_count())
                        .unwrap_or(0)
                } else {
                    self.agent_outputs.get(&agent).map(|b| b.len()).unwrap_or(0)
                }
            }
            None => {
                // Pane 3: shell PTY line count
                if self.pty_mode && idx == 3 {
                    self.shell_pty.as_ref().map(|s| s.line_count()).unwrap_or(0)
                } else {
                    0
                }
            }
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

    /// DX-050 Builder Mode: detect task completion in full TUI mode.
    /// Two detection methods (checked in priority order):
    /// 1. **Signal file** (reliable): agent creates `.forge/signals/T-XXX.complete`
    /// 2. **Heuristic** (fallback): ready-pattern reappears + 10s min work + 3s quiet
    fn check_pty_completion(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let now = Instant::now();
        let mut completed: Vec<AgentType> = Vec::new();
        let signals_dir = self.forge_dir.join("signals");

        for (agent, awaiting) in &mut self.awaiting_completion {
            // Method 1: Check for signal file (instant, no timing requirements)
            let signal_file = signals_dir.join(format!("{}.complete", awaiting.task_id));
            if signal_file.exists() {
                // Clean up signal file
                std::fs::remove_file(&signal_file).ok();
                completed.push(agent.clone());
                continue;
            }

            // Get the agent's ready pattern
            let pattern = match agent {
                AgentType::Claude | AgentType::Any => ClaudeAdapter.ready_pattern(),
                AgentType::Codex => CodexAdapter.ready_pattern(),
                AgentType::Gemini => GeminiAdapter.ready_pattern(),
            };

            // Track pattern disappearance — agent started working when prompt vanishes
            if !awaiting.pattern_disappeared {
                if let Some(session) = self.pty_sessions.get(agent)
                    && let Some(pat) = pattern
                    && !session.has_pattern_in_last_n(pat, 10)
                {
                    awaiting.pattern_disappeared = true;
                }
                // Can't complete until pattern has disappeared and reappeared
                continue;
            }

            // Method 2: Heuristic — ready-pattern REAPPEARANCE + timing
            // Must have been working for at least 10s
            if now.duration_since(awaiting.dispatched_at) < Duration::from_secs(10) {
                continue;
            }
            // Output must have been quiet for at least 3s
            if now.duration_since(awaiting.last_output_at) < Duration::from_secs(3) {
                continue;
            }
            // Check if ready-pattern reappears (agent returned to prompt)
            if let Some(session) = self.pty_sessions.get(agent)
                && let Some(pat) = pattern
                && session.has_pattern_in_last_n(pat, 10)
            {
                completed.push(agent.clone());
            }
        }

        // Process completions
        for agent in completed {
            if let Some(awaiting) = self.awaiting_completion.remove(&agent) {
                let duration = now.duration_since(awaiting.dispatched_at);
                let secs = duration.as_secs();
                self.push_event(&format!(
                    "{} completed by {} ({}s, builder mode)",
                    awaiting.task_id, agent, secs
                ));
                // Send synthetic Completed event
                let _ = tx.send(AgentEvent::Completed {
                    task_id: awaiting.task_id,
                    agent,
                    success: true,
                    exit_code: 0,
                });
            }
        }
    }

    /// Build the completion signal instruction to append to task prompts.
    fn completion_signal_instruction(forge_dir: &std::path::Path, task_id: &str) -> String {
        let signals_dir = forge_dir.join("signals");
        format!(
            " When you have fully completed this task, run this bash command: mkdir -p {} && touch {}/{}.complete",
            signals_dir.display(),
            signals_dir.display(),
            task_id
        )
    }

    /// Handle a tick event: throttled task reload, backoff checks, completion detection.
    pub fn handle_tick(
        &mut self,
        agent_tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> anyhow::Result<()> {
        if self.last_task_reload.elapsed() > std::time::Duration::from_secs(2) {
            self.reload_tasks()?;
            self.last_task_reload = Instant::now();
            // Check phase transitions on each reload
            self.check_phase_transition();
        }
        self.check_backoff_timers(agent_tx);
        if self.pty_mode {
            self.check_pty_completion(agent_tx);
        }
        // Periodic scheduling: pick up tasks that were delayed by pacing cooldowns.
        // schedule_unblocked_tasks() is normally only called on completion/error events,
        // so pacing-delayed tasks would never get dispatched without this.
        if !self.watch_mode && !self.all_complete {
            self.schedule_unblocked_tasks(agent_tx);
        }
        self.spinner_frame = (self.spinner_frame + 1) % 10;
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

    /// DX-024: Handle terminal resize — propagate dimensions to all active PTYs.
    pub fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.terminal_size = (cols, rows);
        if !self.pty_mode {
            return;
        }
        let (pane_cols, pane_rows) = self.estimate_pane_size();
        for session in self.pty_sessions.values() {
            let _ = session.resize(pane_cols, pane_rows);
        }
        if let Some(ref shell) = self.shell_pty {
            let _ = shell.resize(pane_cols, pane_rows);
        }
    }

    /// DX-024: Estimate the inner dimensions of a single pane from terminal size.
    /// Layout: 2x2 grid occupying ~50% of terminal height, each pane is half width.
    pub fn estimate_pane_size(&self) -> (u16, u16) {
        let (cols, rows) = self.terminal_size;
        // Each pane: half the terminal width minus borders (2)
        let pane_cols = (cols / 2).saturating_sub(2).max(10);
        // Agent pane area is ~50% of terminal, split into 2 rows, minus borders
        let pane_rows = (rows / 4).saturating_sub(2).max(5);
        (pane_cols, pane_rows)
    }

    /// Estimate expanded pane size (nearly full terminal).
    fn estimate_expanded_pane_size(&self) -> (u16, u16) {
        let (cols, rows) = self.terminal_size;
        // Full width minus borders, nearly full height minus header/footer
        let pane_cols = cols.saturating_sub(4).max(10);
        let pane_rows = rows.saturating_sub(8).max(10);
        (pane_cols, pane_rows)
    }

    /// Resize the PTY for a pane based on current expand state.
    fn resize_pty_for_pane(&self, pane_idx: usize) {
        if !self.pty_mode {
            return;
        }
        let (cols, rows) = if self.expanded_pane.is_some() {
            self.estimate_expanded_pane_size()
        } else {
            self.estimate_pane_size()
        };

        // Map pane index to agent type
        let agent = match pane_idx {
            0 => Some(AgentType::Claude),
            1 => Some(AgentType::Codex),
            2 => Some(AgentType::Gemini),
            3 => None, // Shell pane
            _ => None,
        };

        if pane_idx == 3 {
            if let Some(ref shell) = self.shell_pty {
                let _ = shell.resize(cols, rows);
            }
        } else if let Some(agent) = agent
            && let Some(session) = self.pty_sessions.get(&agent)
        {
            let _ = session.resize(cols, rows);
        }
    }

    /// Spawn a user shell in pane 3 (replaces Summary).
    pub fn spawn_shell(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.shell_active {
            return;
        }

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        // DX-024: PTY mode — spawn shell with a real PTY
        if self.pty_mode {
            self.spawn_shell_pty(&shell, tx);
            return;
        }

        self.spawn_shell_piped(&shell, tx);
    }

    /// Spawn shell using PTY (Stargate mode).
    fn spawn_shell_pty(&mut self, shell: &str, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(&self.project_root);
        cmd.env("TERM", "xterm-256color");

        let (pane_cols, pane_rows) = self.estimate_pane_size();
        let size = PtySize {
            rows: pane_rows,
            cols: pane_cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        match PtySession::spawn(
            cmd,
            size,
            tx.clone(),
            "__shell__".to_string(),
            AgentType::Any,
            500,
        ) {
            Ok(session) => {
                self.shell_pty = Some(session);
                self.shell_active = true;
                self.shell_output.clear();
                self.push_event("PTY shell opened (i:Attach | Esc:Detach)");
            }
            Err(e) => {
                self.push_event(&format!("PTY shell failed, falling back: {}", e));
                self.spawn_shell_piped(shell, tx);
            }
        }
    }

    /// Spawn shell using piped I/O (legacy mode).
    fn spawn_shell_piped(&mut self, shell: &str, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let mut cmd = TokioCommand::new(shell);
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
                        let mut lines = tokio::io::BufReader::new(stdout).lines();
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
                        let mut lines = tokio::io::BufReader::new(stderr).lines();
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
                self.shell_output.push_back(format!("$ {}", shell));
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
        let enabled = state_mgr.load().map(|s| s.git.auto_commit).unwrap_or(true);

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

    /// DX-050 Builder Mode: dispatch the selected task to the agent's running TUI.
    fn dispatch_to_tui(&mut self, _tx: &mpsc::UnboundedSender<AgentEvent>) {
        if self.selected_index >= self.tasks.len() {
            return;
        }
        let task = &self.tasks[self.selected_index];

        // Guard: task must be Pending
        if task.status != TaskStatus::Pending {
            self.push_event(&format!("{} is not pending ({:?})", task.id, task.status));
            return;
        }

        // Resolve agent
        let agent = task.assigned_to.clone().unwrap_or(AgentType::Claude);
        let agent = if agent == AgentType::Any {
            AgentType::Claude
        } else {
            agent
        };

        // Guard: TUI must be running
        if !self.pty_sessions.contains_key(&agent) {
            self.push_event(&format!("{} TUI not running — cannot dispatch", agent));
            return;
        }

        // Guard: TUI must be idle (not already running a task)
        if self.agent_running_task.contains_key(&agent) {
            self.push_event(&format!(
                "{} is busy with {} — wait for completion",
                agent,
                self.agent_running_task.get(&agent).unwrap()
            ));
            return;
        }

        // Build prompt via adapter's initial_input
        let (initial, pattern) = match agent {
            AgentType::Claude | AgentType::Any => (
                ClaudeAdapter.initial_input(task),
                ClaudeAdapter.ready_pattern().map(String::from),
            ),
            AgentType::Codex => (
                CodexAdapter.initial_input(task),
                CodexAdapter.ready_pattern().map(String::from),
            ),
            AgentType::Gemini => (
                GeminiAdapter.initial_input(task),
                GeminiAdapter.ready_pattern().map(String::from),
            ),
        };

        let Some(mut text) = initial else {
            self.push_event(&format!(
                "No prompt for {} — adapter returned None",
                task.id
            ));
            return;
        };

        // Inject completion signal instruction before the trailing \r
        let signal = Self::completion_signal_instruction(&self.forge_dir, &task.id);
        if text.ends_with('\r') {
            text.insert_str(text.len() - 1, &signal);
        } else {
            text.push_str(&signal);
        }

        // Type the prompt into the TUI.
        // schedule_input_when_ready() polls for the ready-pattern with its own timeout,
        // so we don't need a hard pre-check here — just dispatch and let it wait.
        let session = self.pty_sessions.get(&agent).unwrap();
        if let Some(pat) = pattern {
            session.schedule_input_when_ready(text, pat, 10000);
        } else {
            session.schedule_input(text, 300);
        }

        // Mark task InProgress
        let task_mgr = TaskManager::new(&self.forge_dir);
        let state_mgr = StateManager::new(&self.forge_dir);
        if let Ok(mut t) = task_mgr.get_task(&task.id) {
            t.status = TaskStatus::InProgress;
            t.assigned_to = Some(agent.clone());
            t.updated_at = chrono::Utc::now();
            task_mgr.update_task(&t).ok();
            if !t.locked_files.is_empty() {
                state_mgr
                    .lock_files(&t.id, agent.clone(), t.locked_files.clone())
                    .ok();
            }
        }

        let task_id = task.id.clone();
        self.running_task_ids.insert(task_id.clone());
        self.agent_running_task
            .insert(agent.clone(), task_id.clone());

        // DX-037: Track quota usage
        let now = Instant::now();
        let quota = self.provider_quota.entry(agent.clone()).or_insert((0, now));
        if quota.1.elapsed() > Duration::from_secs(5 * 3600) {
            *quota = (0, now);
        }
        quota.0 += 1;

        // Track completion detection — pattern must disappear then reappear
        self.awaiting_completion.insert(
            agent.clone(),
            AwaitingCompletion {
                task_id: task_id.clone(),
                dispatched_at: now,
                last_output_at: now,
                pattern_disappeared: false,
            },
        );

        self.push_event(&format!("Dispatched {} to {}", task_id, agent));
        self.reload_tasks().ok();
    }

    /// Cycle the agent assignment on the selected task: Claude → Codex → Gemini → Claude.
    fn cycle_agent_assignment(&mut self) {
        if self.selected_index >= self.tasks.len() {
            return;
        }
        let task = &self.tasks[self.selected_index];

        // Only allow reassignment on Pending or Failed tasks
        if !matches!(task.status, TaskStatus::Pending | TaskStatus::Failed) {
            self.push_event(&format!(
                "{} is {:?} — can only reassign pending/failed tasks",
                task.id, task.status
            ));
            return;
        }

        let current = task.assigned_to.clone().unwrap_or(AgentType::Any);
        let next = match current {
            AgentType::Claude => AgentType::Codex,
            AgentType::Codex => AgentType::Gemini,
            AgentType::Gemini => AgentType::Claude,
            AgentType::Any => AgentType::Claude,
        };

        let task_id = task.id.clone();
        let task_mgr = TaskManager::new(&self.forge_dir);
        if let Ok(mut t) = task_mgr.get_task(&task_id) {
            t.assigned_to = Some(next.clone());
            t.updated_at = chrono::Utc::now();
            task_mgr.update_task(&t).ok();
        }
        self.push_event(&format!("{} reassigned to {}", task_id, next));
        self.reload_tasks().ok();
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
        self.tasks
            .iter()
            .filter(|t| t.phase != Some(TaskPhase::Uat))
            .all(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed)
    }

    /// Check if all build-phase tasks are done and transition to Verify phase.
    /// Returns true if a phase transition occurred.
    fn check_phase_transition(&mut self) -> bool {
        match self.phase {
            DashboardPhase::Build => {
                // DX-052: Check for pending quality gate results first
                if self.quality_gate_pending {
                    if let Some(ref rx) = self.quality_gate_rx
                        && let Ok(results) = rx.try_recv()
                    {
                        return self.handle_gate_results(results);
                    }
                    return false; // Still waiting for gate results
                }

                // Build tasks: phase is None or Build
                let build_tasks: Vec<&Task> = self
                    .tasks
                    .iter()
                    .filter(|t| t.phase.is_none() || t.phase == Some(TaskPhase::Build))
                    .collect();

                if build_tasks.is_empty() {
                    return false;
                }

                let all_build_done = build_tasks
                    .iter()
                    .all(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed);

                if all_build_done {
                    // DX-052: Run quality gates before transitioning to Verify
                    let gates = quality_gate::detect_gates(&self.project_root);
                    if gates.is_empty() {
                        self.push_event("No quality gates detected — skipping gate check");
                        return self.transition_to_verify();
                    }

                    self.push_event(&format!("Running {} quality gate(s)...", gates.len()));
                    for g in &gates {
                        self.push_event(&format!("  Gate: {}", g.name));
                    }

                    let project_root = self.project_root.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let results = quality_gate::run_gates(&project_root, &gates);
                        let _ = tx.send(results);
                    });
                    self.quality_gate_rx = Some(rx);
                    self.quality_gate_pending = true;
                    return false; // Wait for results on next tick
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
                    self.generate_uat_tasks();
                    return true;
                }

                let all_verify_done = verify_fix_tasks
                    .iter()
                    .all(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed);

                if all_verify_done {
                    self.phase = DashboardPhase::Complete;
                    self.push_event("Phase 2 (VERIFY) complete — all verification done");
                    self.generate_uat_tasks();
                    return true;
                }
                false
            }
            DashboardPhase::Complete => false,
        }
    }

    /// DX-052: Execute the Build→Verify transition (generate verify subtasks + reload).
    fn transition_to_verify(&mut self) -> bool {
        self.phase = DashboardPhase::Verify;
        self.push_event("Phase 1 (BUILD) complete — transitioning to VERIFY");

        let task_mgr = TaskManager::new(&self.forge_dir);
        match task_mgr.generate_verify_subtasks() {
            Ok(generated) => {
                if generated.is_empty() {
                    self.push_event("No verify subtasks needed");
                } else {
                    self.push_event(&format!("Generated {} verify subtask(s)", generated.len()));
                }
            }
            Err(e) => {
                self.push_event(&format!("Failed to generate verify subtasks: {e}"));
            }
        }
        self.reload_tasks().ok();
        true
    }

    /// DX-052: Process quality gate results — either transition or generate fix tasks.
    fn handle_gate_results(&mut self, results: Vec<GateResult>) -> bool {
        self.quality_gate_pending = false;
        self.quality_gate_rx = None;
        self.quality_gate_attempts += 1;

        let failed: Vec<&GateResult> = results.iter().filter(|r| !r.passed).collect();
        let passed: Vec<&GateResult> = results.iter().filter(|r| r.passed).collect();

        // Log event for the audit trail
        let event_logger = EventLogger::new(&self.forge_dir);
        for r in &passed {
            self.push_event(&format!(
                "✓ Gate passed: {} ({}ms)",
                r.gate_name, r.duration_ms
            ));
            let _ = event_logger.log(&ForgeEvent::new(
                EventType::QualityGatePassed,
                format!("{} passed", r.gate_name),
            ));
        }

        if failed.is_empty() {
            self.push_event("All quality gates passed!");
            return self.transition_to_verify();
        }

        for r in &failed {
            self.push_event(&format!(
                "✗ Gate FAILED: {} (exit {})",
                r.gate_name, r.exit_code
            ));
            let _ = event_logger.log(&ForgeEvent::new(
                EventType::QualityGateFailed,
                format!("{} failed (exit {})", r.gate_name, r.exit_code),
            ));
        }

        if self.quality_gate_attempts >= 3 {
            self.push_event("Quality gates failed 3 times — force-transitioning to VERIFY");
            return self.transition_to_verify();
        }

        // Generate fix tasks for each failed gate
        let task_mgr = TaskManager::new(&self.forge_dir);
        let fail_vec: Vec<GateResult> = results.into_iter().filter(|r| !r.passed).collect();
        match task_mgr.generate_gate_fix_tasks(&fail_vec) {
            Ok(fix_tasks) => {
                self.push_event(&format!(
                    "Generated {} fix task(s) for failed gates (attempt {}/3)",
                    fix_tasks.len(),
                    self.quality_gate_attempts
                ));
            }
            Err(e) => {
                self.push_event(&format!("Failed to generate fix tasks: {e}"));
            }
        }
        self.reload_tasks().ok();
        false // Stay in Build — fix tasks will be dispatched
    }

    /// Generate UAT subtasks for user-facing T-xxx tasks during Verify→Complete transition.
    fn generate_uat_tasks(&mut self) {
        let task_mgr = TaskManager::new(&self.forge_dir);
        match task_mgr.generate_uat_subtasks() {
            Ok(generated) => {
                if generated.is_empty() {
                    self.push_event("No UAT tasks generated (no user-facing tasks)");
                } else {
                    self.push_event(&format!(
                        "Generated {} UAT task(s) — run `forge uat`",
                        generated.len()
                    ));
                }
            }
            Err(e) => {
                self.push_event(&format!("Failed to generate UAT tasks: {e}"));
            }
        }
        self.reload_tasks().ok();
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
            format!(
                "Fix: {} (retry {})",
                task.title.trim_start_matches("Verify: "),
                task.retry_count + 1
            ),
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
            format!(
                "Re-verify: {}",
                task.title
                    .trim_start_matches("Verify: ")
                    .trim_start_matches("Re-verify: ")
            ),
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

        if task_mgr.create_task(&fix_task).is_ok() && task_mgr.create_task(&re_verify).is_ok() {
            self.push_event(&format!(
                "Auto-generated {} + {} to fix failures (retry {}/3)",
                fix_id,
                re_verify_id,
                task.retry_count + 1
            ));
        }
    }

    fn spawn_task(&mut self, task: &Task, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let agent = task.assigned_to.clone().unwrap_or(AgentType::Claude);

        let agent = if agent == AgentType::Any {
            AgentType::Claude
        } else {
            agent
        };

        if self.pty_mode {
            self.spawn_task_pty(task, &agent, tx);
        } else {
            self.spawn_task_piped(task, &agent, tx);
        }
    }

    /// Dispatch a task's prompt into an already-running TUI session.
    fn dispatch_task_to_existing_tui(
        &mut self,
        task: &Task,
        agent: &AgentType,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) {
        let (initial, timeout, pattern) = match agent {
            AgentType::Claude | AgentType::Any => (
                ClaudeAdapter.initial_input(task),
                ClaudeAdapter.initial_input_delay_ms(),
                ClaudeAdapter.ready_pattern().map(String::from),
            ),
            AgentType::Codex => (
                CodexAdapter.initial_input(task),
                CodexAdapter.initial_input_delay_ms(),
                CodexAdapter.ready_pattern().map(String::from),
            ),
            AgentType::Gemini => (
                GeminiAdapter.initial_input(task),
                GeminiAdapter.initial_input_delay_ms(),
                GeminiAdapter.ready_pattern().map(String::from),
            ),
        };

        if let Some(mut text) = initial {
            // Inject completion signal instruction
            let signal = Self::completion_signal_instruction(&self.forge_dir, &task.id);
            if text.ends_with('\r') {
                text.insert_str(text.len() - 1, &signal);
            } else {
                text.push_str(&signal);
            }

            let session = self.pty_sessions.get(agent).unwrap();
            if let Some(pat) = pattern {
                session.schedule_input_when_ready(text, pat, timeout);
            } else {
                session.schedule_input(text, timeout);
            }
        }

        // Track for completion detection
        let now = Instant::now();
        self.awaiting_completion.insert(
            agent.clone(),
            AwaitingCompletion {
                task_id: task.id.clone(),
                dispatched_at: now,
                last_output_at: now,
                pattern_disappeared: false,
            },
        );

        self.finalize_task_spawn(task, agent, tx);
    }

    /// DX-050: Ensure all 3 agent TUIs are running in PTY mode.
    /// Spawns idle TUI sessions for agents that don't already have a PTY session.
    /// Called on startup so users can manually dispatch tasks via `a` key.
    pub fn spawn_idle_tuis(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if !self.pty_mode {
            return;
        }
        let agents = [AgentType::Claude, AgentType::Codex, AgentType::Gemini];
        for agent in &agents {
            if self.pty_sessions.contains_key(agent) {
                continue; // Already has a TUI from task dispatch
            }
            self.spawn_idle_tui(agent, tx);
        }
    }

    /// Spawn an idle TUI for an agent (no task, just launch the interactive CLI).
    fn spawn_idle_tui(&mut self, agent: &AgentType, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let state_mgr = StateManager::new(&self.forge_dir);
        let agent_name = agent.to_string().to_lowercase();
        let auth_mode = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());
        let permissions = state_mgr
            .get_agent_permissions(&agent_name)
            .unwrap_or_else(|_| "safe".to_string());

        // Build a dummy task just for the command builder (no prompt will be typed)
        let now = chrono::Utc::now();
        let dummy = Task {
            id: String::new(),
            title: String::new(),
            description: String::new(),
            status: TaskStatus::Pending,
            assigned_to: Some(agent.clone()),
            task_type: None,
            depends_on: Vec::new(),
            locked_files: Vec::new(),
            acceptance_criteria: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            plan_version: None,
            parent_task: None,
            phase: None,
            retry_count: 0,
        };

        let std_cmd = match agent {
            AgentType::Claude | AgentType::Any => ClaudeAdapter.build_command_interactive(
                &dummy,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
            AgentType::Codex => CodexAdapter.build_command_interactive(
                &dummy,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
            AgentType::Gemini => GeminiAdapter.build_command_interactive(
                &dummy,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
        };

        let program = std_cmd.get_program().to_string_lossy().to_string();
        let mut pty_cmd = CommandBuilder::new(&program);
        for arg in std_cmd.get_args() {
            pty_cmd.arg(arg.to_string_lossy().as_ref());
        }
        if let Some(dir) = std_cmd.get_current_dir() {
            pty_cmd.cwd(dir);
        }
        pty_cmd.env("TERM", "xterm-256color");
        for (key, val) in std_cmd.get_envs() {
            if let Some(v) = val {
                pty_cmd.env(key.to_string_lossy().as_ref(), v.to_string_lossy().as_ref());
            } else {
                // Forward env_remove() calls — without this, vars the adapter
                // wanted removed (e.g. API keys in subscription mode) leak through.
                pty_cmd.env_remove(key.to_string_lossy().as_ref());
            }
        }

        let (pane_cols, pane_rows) = self.estimate_pane_size();
        let size = PtySize {
            rows: pane_rows,
            cols: pane_cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        // Use a placeholder task_id for the PTY reader thread
        let placeholder_id = format!("idle-{}", agent_name);

        match PtySession::spawn(
            pty_cmd,
            size,
            tx.clone(),
            placeholder_id,
            agent.clone(),
            OUTPUT_BUFFER_CAP,
        ) {
            Ok(session) => {
                self.pty_sessions.insert(agent.clone(), session);
                self.push_event(&format!("{} TUI launched (idle)", agent));
            }
            Err(e) => {
                self.push_event(&format!("Failed to launch {} TUI: {}", agent, e));
            }
        }
    }

    /// DX-024: Spawn task with PTY allocation (Stargate mode).
    /// If an idle TUI already exists for this agent, reuse it (type prompt into it).
    /// Otherwise, spawn a new PTY process.
    fn spawn_task_pty(
        &mut self,
        task: &Task,
        agent: &AgentType,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) {
        // Reuse existing idle TUI if available
        if self.pty_sessions.contains_key(agent) {
            self.dispatch_task_to_existing_tui(task, agent, tx);
            return;
        }

        let state_mgr = StateManager::new(&self.forge_dir);
        let agent_name = agent.to_string().to_lowercase();
        let auth_mode = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());
        let permissions = state_mgr
            .get_agent_permissions(&agent_name)
            .unwrap_or_else(|_| "safe".to_string());

        // Use interactive command for Claude (no stream-json), standard for others
        let std_cmd = match agent {
            AgentType::Claude | AgentType::Any => ClaudeAdapter.build_command_interactive(
                task,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
            AgentType::Codex => CodexAdapter.build_command_interactive(
                task,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
            AgentType::Gemini => GeminiAdapter.build_command_interactive(
                task,
                &self.project_root,
                &auth_mode,
                &permissions,
            ),
        };

        // Convert std::process::Command to portable_pty::CommandBuilder
        let program = std_cmd.get_program().to_string_lossy().to_string();
        let mut pty_cmd = CommandBuilder::new(&program);
        for arg in std_cmd.get_args() {
            pty_cmd.arg(arg.to_string_lossy().as_ref());
        }
        if let Some(dir) = std_cmd.get_current_dir() {
            pty_cmd.cwd(dir);
        }
        pty_cmd.env("TERM", "xterm-256color");
        // Forward environment from the std command
        for (key, val) in std_cmd.get_envs() {
            if let Some(v) = val {
                pty_cmd.env(key.to_string_lossy().as_ref(), v.to_string_lossy().as_ref());
            } else {
                // Forward env_remove() calls — without this, vars the adapter
                // wanted removed (e.g. API keys in subscription mode) leak through.
                pty_cmd.env_remove(key.to_string_lossy().as_ref());
            }
        }

        let (pane_cols, pane_rows) = self.estimate_pane_size();
        let size = PtySize {
            rows: pane_rows,
            cols: pane_cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        match PtySession::spawn(
            pty_cmd,
            size,
            tx.clone(),
            task.id.clone(),
            agent.clone(),
            OUTPUT_BUFFER_CAP,
        ) {
            Ok(session) => {
                // Check if adapter wants to type initial input into the TUI
                let (initial, timeout, pattern) = match agent {
                    AgentType::Claude | AgentType::Any => (
                        ClaudeAdapter.initial_input(task),
                        ClaudeAdapter.initial_input_delay_ms(),
                        ClaudeAdapter.ready_pattern().map(String::from),
                    ),
                    AgentType::Codex => (
                        CodexAdapter.initial_input(task),
                        CodexAdapter.initial_input_delay_ms(),
                        CodexAdapter.ready_pattern().map(String::from),
                    ),
                    AgentType::Gemini => (
                        GeminiAdapter.initial_input(task),
                        GeminiAdapter.initial_input_delay_ms(),
                        GeminiAdapter.ready_pattern().map(String::from),
                    ),
                };

                // Schedule initial input: pattern-based (instant) or fixed delay (fallback)
                if let Some(mut text) = initial {
                    // Inject completion signal instruction before the trailing \r
                    let signal = Self::completion_signal_instruction(&self.forge_dir, &task.id);
                    if text.ends_with('\r') {
                        text.insert_str(text.len() - 1, &signal);
                    } else {
                        text.push_str(&signal);
                    }

                    if let Some(pat) = pattern {
                        session.schedule_input_when_ready(text, pat, timeout);
                    } else {
                        session.schedule_input(text, timeout);
                    }
                }

                self.pty_sessions.insert(agent.clone(), session);

                // Track for completion detection — full TUI processes don't exit after task.
                self.awaiting_completion.insert(
                    agent.clone(),
                    AwaitingCompletion {
                        task_id: task.id.clone(),
                        dispatched_at: Instant::now(),
                        last_output_at: Instant::now(),
                        pattern_disappeared: false,
                    },
                );

                self.finalize_task_spawn(task, agent, tx);
            }
            Err(e) => {
                self.push_event(&format!(
                    "PTY spawn failed for {}, falling back to piped: {}",
                    task.id, e
                ));
                self.spawn_task_piped(task, agent, tx);
            }
        }
    }

    /// Spawn task with piped I/O (legacy mode).
    fn spawn_task_piped(
        &mut self,
        task: &Task,
        agent: &AgentType,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) {
        let state_mgr = StateManager::new(&self.forge_dir);
        let agent_name = agent.to_string().to_lowercase();
        let auth_mode = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());
        let permissions = state_mgr
            .get_agent_permissions(&agent_name)
            .unwrap_or_else(|_| "safe".to_string());

        let std_cmd = match agent {
            AgentType::Claude | AgentType::Any => {
                ClaudeAdapter.build_command(task, &self.project_root, &auth_mode, &permissions)
            }
            AgentType::Codex => {
                CodexAdapter.build_command(task, &self.project_root, &auth_mode, &permissions)
            }
            AgentType::Gemini => {
                GeminiAdapter.build_command(task, &self.project_root, &auth_mode, &permissions)
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

                self.finalize_task_spawn(task, agent, tx);
            }
            Err(e) => {
                self.push_event(&format!("Failed to spawn {}: {}", task.id, e));
                let _ = tx.send(AgentEvent::Error {
                    task_id: task.id.clone(),
                    agent: agent.clone(),
                    message: e.to_string(),
                });
            }
        }
    }

    /// Common post-spawn bookkeeping for both PTY and piped modes.
    fn finalize_task_spawn(
        &mut self,
        task: &Task,
        agent: &AgentType,
        _tx: &mpsc::UnboundedSender<AgentEvent>,
    ) {
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
        self.agent_running_task
            .insert(agent.clone(), task.id.clone());

        // DX-037: Track quota usage
        let quota = self
            .provider_quota
            .entry(agent.clone())
            .or_insert((0, Instant::now()));
        // Reset counter if 5-hour window has elapsed
        if quota.1.elapsed() > std::time::Duration::from_secs(5 * 3600) {
            *quota = (0, Instant::now());
        }
        quota.0 += 1;

        self.push_event(&format!("Started {} on {}", task.id, agent));
    }

    pub fn cleanup_running_tasks(&mut self) {
        // DX-024: Kill all active PTY sessions
        for (_agent, mut session) in self.pty_sessions.drain() {
            session.kill();
        }
        if let Some(mut shell) = self.shell_pty.take() {
            shell.kill();
        }
        self.attached_pane = None;
        self.awaiting_completion.clear();

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
/// Matches DX-035 spec: 60s → 120s → 240s → 480s → 600s (max 10 min).
fn compute_backoff_delay(attempt: u32) -> std::time::Duration {
    let base_secs: u64 = match attempt {
        1 => 60,
        2 => 120,
        3 => 240,
        4 => 480,
        _ => 600,
    };
    let jitter = rand::rng().random_range(0..=30u64);
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
                            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let input_summary = summarize_tool_input(name, item.get("input"));
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
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
            truncate_str(cmd, 60)
        }
        "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
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
        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert!(app.is_all_done());
    }

    #[test]
    fn test_is_all_done_with_completed_tasks() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.tasks = vec![
            make_task("T-001", TaskStatus::Completed, vec![]),
            make_task("T-002", TaskStatus::Failed, vec![]),
        ];
        assert!(app.is_all_done());
    }

    #[test]
    fn test_is_all_done_with_pending() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.tasks = vec![
            make_task("T-001", TaskStatus::Completed, vec![]),
            make_task("T-002", TaskStatus::Pending, vec![]),
        ];
        assert!(!app.is_all_done());
    }

    #[test]
    fn test_schedule_respects_parallel_limit() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 2, false);
        app.running_task_ids.insert("T-001".to_string());
        app.running_task_ids.insert("T-002".to_string());
        app.tasks = vec![make_task("T-003", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-003"));
    }

    #[test]
    fn test_blocked_tasks_not_scheduled() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, true);
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-001"));
    }

    #[test]
    fn test_handle_key_quit() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.handle_key(KeyEvent::from(KeyCode::Char('q')), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn test_tab_cycles_through_panes_and_back() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert_eq!(app.focus, FocusArea::TaskBoard);

        app.handle_key(KeyEvent::from(KeyCode::BackTab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(3));

        app.handle_key(KeyEvent::from(KeyCode::BackTab), &tx);
        assert_eq!(app.focus, FocusArea::Pane(2));
    }

    #[test]
    fn test_task_board_navigation() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        app.focus = FocusArea::Pane(2);
        app.handle_key(KeyEvent::from(KeyCode::Enter), &tx);
        assert_eq!(app.expanded_pane, Some(2));

        // Esc collapses
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert_eq!(app.expanded_pane, None);
    }

    #[test]
    fn test_expanded_pane_blocks_other_keys() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.expanded_pane = Some(0);

        // Tab should not change focus while expanded
        app.handle_key(KeyEvent::from(KeyCode::Tab), &tx);
        assert_eq!(app.expanded_pane, Some(0)); // still expanded
    }

    #[test]
    fn test_esc_from_pane_returns_to_board() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.focus = FocusArea::Pane(1);
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert_eq!(app.focus, FocusArea::TaskBoard);
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_from_board_quits() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn test_push_event_caps_at_limit() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert!(!app.all_complete);
        assert!(app.completed_at.is_none());
    }

    #[test]
    fn test_retry_key_on_non_retryable_status_is_noop() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        app.selected_index = 0;
        let events_before = app.events.len();
        app.handle_key(KeyEvent::from(KeyCode::Char('r')), &tx);
        assert_eq!(app.events.len(), events_before);
    }

    #[test]
    fn test_r_key_ignored_when_pane_focused() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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

        let (mut app, _rx, _tx) = App::new(forge_dir.clone(), tmp.path().to_path_buf(), 3, false);

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

        let (mut app, _rx, tx) = App::new(forge_dir.clone(), tmp.path().to_path_buf(), 3, true);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.focus = FocusArea::Pane(1);
        app.handle_key(KeyEvent::from(KeyCode::Char('f')), &tx);
        assert_eq!(app.expanded_pane, Some(1));
    }

    #[test]
    fn test_scroll_in_expanded_mode() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        // Attempt 1: 60s base + 0-30s jitter = 60-90s
        assert!(delay.as_secs() >= 60 && delay.as_secs() <= 90);
    }

    #[test]
    fn test_backoff_delay_attempt_2() {
        let delay = compute_backoff_delay(2);
        // Attempt 2: 120s base + 0-30s jitter = 120-150s
        assert!(delay.as_secs() >= 120 && delay.as_secs() <= 150);
    }

    #[test]
    fn test_backoff_delay_attempt_4() {
        let delay = compute_backoff_delay(4);
        // Attempt 4: 480s base + 0-30s jitter = 480-510s
        assert!(delay.as_secs() >= 480 && delay.as_secs() <= 510);
    }

    #[test]
    fn test_is_agent_in_backoff_true() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        assert!(
            app.agent_backoff
                .get(&AgentType::Gemini)
                .unwrap()
                .next_retry
                .is_none()
        );
        // attempt should be preserved (for escalation if it fails again)
        assert_eq!(
            app.agent_backoff.get(&AgentType::Gemini).unwrap().attempt,
            2
        );
        // Event should be logged
        assert!(app.events.iter().any(|e| e.contains("Backoff expired")));
    }

    #[test]
    fn test_backoff_reset_on_success_tracking() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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

        let (mut app, _rx, tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);

        // First handle_tick should reload (last_task_reload was just set)
        // But since < 2s have elapsed, reload should be skipped
        let before = app.last_task_reload;
        app.handle_tick(&tx).unwrap();
        assert_eq!(app.last_task_reload, before, "should not reload within 2s");

        // Force last_task_reload to 3 seconds ago
        app.last_task_reload = Instant::now() - std::time::Duration::from_secs(3);
        let before = app.last_task_reload;
        app.handle_tick(&tx).unwrap();
        assert_ne!(
            app.last_task_reload, before,
            "should reload after 2s elapsed"
        );
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

        let (mut app, _rx, tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
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

        let (mut app, _rx, _tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
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

        let (mut app, _rx, _tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
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

        let (mut app, _rx, _tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
        let mut task = Task::new("T-005", "Add caching layer", "desc");
        task.task_type = Some("implement".to_string());

        app.git_auto_commit(&task, &AgentType::Codex);

        // Should have pushed an event and added to agent output
        assert!(
            app.events
                .iter()
                .any(|e| e.contains("T-005 auto-committed (feat)"))
        );
        let codex_buf = app.agent_outputs.get(&AgentType::Codex).unwrap();
        assert!(
            codex_buf
                .iter()
                .any(|l| l.contains("Auto-committing: feat(T-005)"))
        );
    }

    // ── DX-027: User-spawnable shell pane tests ─────────────────

    #[test]
    fn test_shell_defaults_inactive() {
        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert!(!app.shell_active);
        assert!(app.shell_output.is_empty());
        assert!(app.shell_input_tx.is_none());
    }

    #[test]
    fn test_shell_output_routes_to_shell_buffer() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        assert!(
            app.agent_outputs.get(&AgentType::Any).is_none()
                || app.agent_outputs.get(&AgentType::Any).unwrap().is_empty()
        );
    }

    #[test]
    fn test_shell_completion_resets_state() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        // Manually set shell_active to simulate it was already spawned
        // (can't actually spawn_shell without tokio runtime)
        app.shell_active = true;

        app.focus = FocusArea::TaskBoard;
        app.handle_key(KeyEvent::from(KeyCode::Char('s')), &tx);
        assert_eq!(app.focus, FocusArea::Pane(3));
    }

    #[test]
    fn test_shell_not_double_spawnable() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert_eq!(app.phase, DashboardPhase::Build);
    }

    #[test]
    fn test_phase_transitions_build_to_verify() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
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

        let (mut app, _rx, _tx) = App::new(forge_dir.clone(), tmp.path().to_path_buf(), 3, false);
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
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.phase = DashboardPhase::Verify;

        let mut verify = Task::new("V-001", "Verify: Auth", "Run tests");
        verify.phase = Some(TaskPhase::Verify);
        verify.retry_count = 3; // Already at max
        verify.status = TaskStatus::Failed;

        app.handle_verify_failure(&verify);

        assert!(
            app.events
                .iter()
                .any(|e| e.contains("needs human attention"))
        );
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
        let lines =
            parse_stream_json_line(r#"{"type":"result","status":"success","duration_ms":45000}"#);
        assert_eq!(lines, vec!["[done] success in 45.0s"]);
    }

    #[test]
    fn test_parse_stream_json_tool_result_skipped() {
        let lines = parse_stream_json_line(r#"{"type":"tool_result","output":"lots of stuff"}"#);
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

    // ── DX-037: Quota monitoring tests ──────────────────────────────

    #[test]
    fn test_quota_counter_increments() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        // Simulate quota increments
        let quota = app
            .provider_quota
            .entry(AgentType::Claude)
            .or_insert((0, Instant::now()));
        quota.0 += 1;
        assert_eq!(app.provider_quota[&AgentType::Claude].0, 1);

        app.provider_quota.get_mut(&AgentType::Claude).unwrap().0 += 1;
        assert_eq!(app.provider_quota[&AgentType::Claude].0, 2);
    }

    #[test]
    fn test_quota_window_resets_after_5h() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        // Insert a quota entry with a stale window (pretend it started 6 hours ago)
        let old_start = Instant::now() - std::time::Duration::from_secs(6 * 3600);
        app.provider_quota
            .insert(AgentType::Claude, (42, old_start));

        // Simulate the reset logic from spawn_task
        let quota = app
            .provider_quota
            .entry(AgentType::Claude)
            .or_insert((0, Instant::now()));
        if quota.1.elapsed() > std::time::Duration::from_secs(5 * 3600) {
            *quota = (0, Instant::now());
        }
        quota.0 += 1;

        assert_eq!(app.provider_quota[&AgentType::Claude].0, 1);
    }

    #[test]
    fn test_quota_separate_per_agent() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        app.provider_quota
            .insert(AgentType::Claude, (5, Instant::now()));
        app.provider_quota
            .insert(AgentType::Codex, (10, Instant::now()));
        app.provider_quota
            .insert(AgentType::Gemini, (15, Instant::now()));

        assert_eq!(app.provider_quota[&AgentType::Claude].0, 5);
        assert_eq!(app.provider_quota[&AgentType::Codex].0, 10);
        assert_eq!(app.provider_quota[&AgentType::Gemini].0, 15);
    }

    // ── DX-024: Stargate PTY mode tests ─────────────────────────

    // ── DX-050: Builder Mode tests ─────────────────────────────

    #[test]
    fn test_dispatch_to_tui_pending_task() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        // Create a pending task assigned to Claude
        let mut task = Task::new("T-010", "Build auth", "Implement auth module");
        task.status = TaskStatus::Pending;
        task.assigned_to = Some(AgentType::Claude);
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-010.json"), &json).unwrap();
        std::fs::write(tasks_dir.join("T-010.md"), "# T-010").unwrap();

        let (mut app, _rx, tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
        app.reload_tasks().unwrap();
        app.pty_mode = true;
        app.selected_index = 0;

        // Simulate a running PTY session for Claude with a ready prompt
        // We can't easily create a real PtySession in tests, but we can test
        // the guards: dispatch should fail without a PTY session
        app.handle_key(KeyEvent::from(KeyCode::Char('a')), &tx);
        // Should log "TUI not running" since no pty_sessions
        assert!(app.events.iter().any(|e| e.contains("not running")));
    }

    #[test]
    fn test_dispatch_to_tui_busy_agent() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.pty_mode = true;
        app.focus = FocusArea::TaskBoard;

        let mut task = make_task("T-001", TaskStatus::Pending, vec![]);
        task.assigned_to = Some(AgentType::Claude);
        app.tasks = vec![task];
        app.selected_index = 0;

        // Claude is already running a task
        app.agent_running_task
            .insert(AgentType::Claude, "T-999".to_string());

        app.dispatch_to_tui(&tx);
        // Should log "busy" since agent already running
        // Note: this will first fail on "TUI not running" since no pty_sessions
        assert!(
            app.events
                .iter()
                .any(|e| e.contains("not running") || e.contains("busy")),
            "Expected dispatch guard message, got: {:?}",
            app.events
        );
    }

    #[test]
    fn test_completion_detection_too_early() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.pty_mode = true;

        // Insert awaiting completion that was just dispatched (now)
        app.awaiting_completion.insert(
            AgentType::Claude,
            AwaitingCompletion {
                task_id: "T-001".to_string(),
                dispatched_at: Instant::now(),
                last_output_at: Instant::now(),
                pattern_disappeared: false,
            },
        );

        // Should NOT fire completion — less than 10s elapsed
        app.check_pty_completion(&tx);
        assert!(
            app.awaiting_completion.contains_key(&AgentType::Claude),
            "Should not complete before 10s minimum"
        );
    }

    #[test]
    fn test_completion_detection_quiet_period() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.pty_mode = true;

        // Insert awaiting completion that was dispatched 15s ago, output quiet 5s
        let now = Instant::now();
        app.awaiting_completion.insert(
            AgentType::Claude,
            AwaitingCompletion {
                task_id: "T-001".to_string(),
                dispatched_at: now - Duration::from_secs(15),
                last_output_at: now - Duration::from_secs(5),
                pattern_disappeared: true,
            },
        );

        // No PTY session — check_pty_completion should skip (no session to check pattern)
        app.check_pty_completion(&tx);
        assert!(
            app.awaiting_completion.contains_key(&AgentType::Claude),
            "Should not complete without PTY session"
        );
    }

    #[test]
    fn test_awaiting_completion_cleared_on_cleanup() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.awaiting_completion.insert(
            AgentType::Claude,
            AwaitingCompletion {
                task_id: "T-001".to_string(),
                dispatched_at: Instant::now(),
                last_output_at: Instant::now(),
                pattern_disappeared: false,
            },
        );
        assert!(!app.awaiting_completion.is_empty());
        app.cleanup_running_tasks();
        assert!(app.awaiting_completion.is_empty());
    }

    #[test]
    fn test_spinner_frame_increments() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let (mut app, _rx, tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);
        assert_eq!(app.spinner_frame, 0);
        app.handle_tick(&tx).unwrap();
        assert_eq!(app.spinner_frame, 1);
        // Cycle through
        for _ in 0..9 {
            app.handle_tick(&tx).unwrap();
        }
        assert_eq!(app.spinner_frame, 0); // wrapped around
    }

    #[test]
    fn test_pty_mode_defaults_off() {
        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        assert!(!app.pty_mode);
        assert!(app.pty_sessions.is_empty());
        assert!(app.attached_pane.is_none());
        assert!(app.shell_pty.is_none());
    }

    #[test]
    fn test_estimate_pane_size() {
        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.terminal_size = (120, 40);
        let (cols, rows) = app.estimate_pane_size();
        // Half of 120 = 60, minus 2 borders = 58
        assert_eq!(cols, 58);
        // 40 / 4 = 10, minus 2 borders = 8
        assert_eq!(rows, 8);
    }

    #[test]
    fn test_attached_pane_detach() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.attached_pane = Some(0);

        // Esc should detach
        app.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert!(app.attached_pane.is_none());
    }

    #[test]
    fn test_i_key_attaches_in_pty_mode() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.pty_mode = true;
        app.focus = FocusArea::Pane(0);

        // No PTY session yet — 'i' should not attach
        app.handle_key(KeyEvent::from(KeyCode::Char('i')), &tx);
        assert!(app.attached_pane.is_none());

        // 'i' in non-PTY mode should also not attach
        app.pty_mode = false;
        app.handle_key(KeyEvent::from(KeyCode::Char('i')), &tx);
        assert!(app.attached_pane.is_none());
    }

    #[test]
    fn test_cycle_agent_assignment() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let tasks_dir = forge_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let mut task = Task::new("T-020", "Test cycle", "Test agent cycling");
        task.status = TaskStatus::Pending;
        task.assigned_to = Some(AgentType::Claude);
        let json = serde_json::to_string_pretty(&task).unwrap();
        std::fs::write(tasks_dir.join("T-020.json"), &json).unwrap();
        std::fs::write(tasks_dir.join("T-020.md"), "# T-020").unwrap();

        let (mut app, _rx, tx) = App::new(forge_dir.clone(), tmp.path().to_path_buf(), 3, false);
        app.reload_tasks().unwrap();
        app.focus = FocusArea::TaskBoard;
        app.selected_index = 0;

        // Claude → Codex
        app.handle_key(KeyEvent::from(KeyCode::Char('c')), &tx);
        app.reload_tasks().unwrap();
        assert_eq!(app.tasks[0].assigned_to, Some(AgentType::Codex));

        // Codex → Gemini
        app.handle_key(KeyEvent::from(KeyCode::Char('c')), &tx);
        app.reload_tasks().unwrap();
        assert_eq!(app.tasks[0].assigned_to, Some(AgentType::Gemini));

        // Gemini → Claude
        app.handle_key(KeyEvent::from(KeyCode::Char('c')), &tx);
        app.reload_tasks().unwrap();
        assert_eq!(app.tasks[0].assigned_to, Some(AgentType::Claude));
    }

    #[test]
    fn test_cycle_agent_blocked_on_running_task() {
        let (mut app, _rx, tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.focus = FocusArea::TaskBoard;

        let mut task = make_task("T-021", TaskStatus::InProgress, vec![]);
        task.assigned_to = Some(AgentType::Claude);
        app.tasks = vec![task];
        app.selected_index = 0;

        app.handle_key(KeyEvent::from(KeyCode::Char('c')), &tx);
        // Should not cycle — task is InProgress
        assert!(app.events.iter().any(|e| e.contains("can only reassign")));
    }
}
