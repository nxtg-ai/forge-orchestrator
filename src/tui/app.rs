use crate::adapters::claude::ClaudeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::adapters::ToolAdapter;
use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskManager, TaskStatus};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

const OUTPUT_BUFFER_CAP: usize = 200;
const EVENT_BUFFER_CAP: usize = 50;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    TaskBoard,
    AgentPanes,
    EventLog,
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
    pub focus_area: FocusArea,
    pub parallel_limit: usize,
    pub watch_mode: bool,
    pub should_quit: bool,
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
            focus_area: FocusArea::TaskBoard,
            parallel_limit,
            watch_mode,
            should_quit: false,
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
                agent, line, ..
            } => {
                let buf = self
                    .agent_outputs
                    .entry(agent)
                    .or_default();
                if buf.len() >= OUTPUT_BUFFER_CAP {
                    buf.pop_front();
                }
                buf.push_back(line);
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

                if let Ok(task) = task_mgr.get_task(&task_id) {
                    let mut updated = task;
                    if success {
                        updated.status = TaskStatus::Completed;
                        updated.completed_at = Some(chrono::Utc::now());
                    } else {
                        updated.status = TaskStatus::Failed;
                    }
                    updated.updated_at = chrono::Utc::now();
                    task_mgr.update_task(&updated).ok();
                    state_mgr.unlock_files(&task_id).ok();

                    let status_str = if success { "completed" } else { "failed" };
                    let event_msg = format!(
                        "{} {} by {} (exit {})",
                        task_id, status_str, agent, exit_code
                    );
                    self.push_event(&event_msg);

                    event_logger
                        .log(
                            &ForgeEvent::new(
                                if success {
                                    EventType::TaskCompleted
                                } else {
                                    EventType::TaskFailed
                                },
                                event_msg,
                            )
                            .with_task(&task_id)
                            .with_agent(agent),
                        )
                        .ok();
                }

                self.reload_tasks()?;
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
                self.schedule_unblocked_tasks(tx);
            }
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Tab => {
                self.focus_area = match self.focus_area {
                    FocusArea::TaskBoard => FocusArea::AgentPanes,
                    FocusArea::AgentPanes => FocusArea::EventLog,
                    FocusArea::EventLog => FocusArea::TaskBoard,
                };
            }
            KeyCode::Up => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
            }
            KeyCode::Down => {
                if self.selected_index + 1 < self.tasks.len() {
                    self.selected_index += 1;
                }
            }
            _ => {}
        }
    }

    pub fn is_all_done(&self) -> bool {
        self.tasks.iter().all(|t| {
            t.status == TaskStatus::Completed || t.status == TaskStatus::Failed
        })
    }

    fn spawn_task(&mut self, task: &Task, tx: &mpsc::UnboundedSender<AgentEvent>) {
        let agent = task
            .assigned_to
            .clone()
            .unwrap_or(AgentType::Claude);

        // Resolve Any -> Claude
        let agent = if agent == AgentType::Any {
            AgentType::Claude
        } else {
            agent
        };

        // Build std::process::Command via adapter
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

        // Convert to tokio command with piped stdout/stderr
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

                // Stream stdout
                if let Some(stdout) = child.stdout.take() {
                    let tx_out = tx.clone();
                    let tid = task_id.clone();
                    let ag = agent.clone();
                    tokio::spawn(stream_lines(stdout, tid, ag, tx_out));
                }

                // Stream stderr
                if let Some(stderr) = child.stderr.take() {
                    tokio::spawn(stream_lines(stderr, task_id2, agent2, tx2));
                }

                // Wait for exit
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

                // Update task to InProgress on disk
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

    fn push_event(&mut self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        let entry = format!("[{}] {}", ts, msg);
        if self.events.len() >= EVENT_BUFFER_CAP {
            self.events.pop_front();
        }
        self.events.push_back(entry);
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
        // Fill running slots
        app.running_task_ids.insert("T-001".to_string());
        app.running_task_ids.insert("T-002".to_string());
        app.tasks = vec![make_task("T-003", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        // Should not have spawned (no slots)
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
        app.completed_task_ids = vec![]; // T-001 not completed
        app.schedule_unblocked_tasks(&tx);
        // T-002 should not be in running (it's blocked and also spawn would fail without forge dir)
        assert!(!app.running_task_ids.contains("T-002"));
    }

    #[test]
    fn test_watch_mode_no_schedule() {
        let (mut app, _rx, tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            true, // watch mode
        );
        app.tasks = vec![make_task("T-001", TaskStatus::Pending, vec![])];
        app.schedule_unblocked_tasks(&tx);
        assert!(!app.running_task_ids.contains("T-001"));
    }

    #[test]
    fn test_handle_key_quit() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        let key = KeyEvent::from(KeyCode::Char('q'));
        app.handle_key(key);
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_key_tab_cycles_focus() {
        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
        assert_eq!(app.focus_area, FocusArea::TaskBoard);

        app.handle_key(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.focus_area, FocusArea::AgentPanes);

        app.handle_key(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.focus_area, FocusArea::EventLog);

        app.handle_key(KeyEvent::from(KeyCode::Tab));
        assert_eq!(app.focus_area, FocusArea::TaskBoard);
    }

    #[test]
    fn test_handle_key_navigation() {
        let (mut app, _rx, _tx) = App::new(
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
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_index, 1);
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_index, 2);
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.selected_index, 2); // can't go past end

        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.selected_index, 1);
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
}
