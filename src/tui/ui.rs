use crate::core::state::StateManager;
use crate::core::task::{AgentType, Task, TaskPhase, TaskStatus};
use crate::tui::app::{
    App, DashboardPhase, FocusArea, MAX_BACKOFF_ATTEMPTS, pane_agent, pane_label,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use std::time::Instant;

pub fn render(f: &mut Frame, app: &App) {
    // If a pane is expanded, render only that pane full-screen + footer
    if let Some(idx) = app.expanded_pane {
        let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).split(f.area());
        render_single_pane(f, app, idx, chunks[0], true);
        render_footer(f, app, chunks[1]);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Percentage(30), // Task board
        Constraint::Percentage(50), // Agent panes 2x2
        Constraint::Percentage(20), // Event log
        Constraint::Length(2),      // Footer (keys + quota)
    ])
    .split(f.area());

    render_task_board(f, app, chunks[0]);
    render_agent_panes(f, app, chunks[1]);
    render_event_log(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    // Split footer into 2 lines: keys + quota (DX-037)
    let footer_chunks =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    // Line 1: Key legend
    let text = if app.focus == FocusArea::Pane(3) && app.shell_active {
        "Esc:Unfocus | Ctrl+D:Close Shell | Type to interact"
    } else if app.all_complete {
        "q:Quit | \u{2191}\u{2193}:Scroll | Tab:Switch Pane | s:Shell"
    } else if app.expanded_pane.is_some() {
        "Esc:Back | \u{2191}\u{2193}:Scroll | Home/End:Jump | q:Quit"
    } else {
        "q:Quit | Tab:Focus | \u{2191}\u{2193}:Navigate | Enter/f:Expand | r:Retry | s:Shell"
    };

    let key_paragraph = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(key_paragraph, footer_chunks[0]);

    // Line 2: Quota monitoring (DX-037)
    let quota_spans = build_quota_spans(app);
    let quota_line = Paragraph::new(Line::from(quota_spans));
    f.render_widget(quota_line, footer_chunks[1]);
}

/// Build quota display spans for each provider (DX-037).
fn build_quota_spans(app: &App) -> Vec<Span<'static>> {
    let state_mgr = StateManager::new(&app.forge_dir);

    let mut spans = Vec::new();
    for (idx, agent_type) in [AgentType::Claude, AgentType::Codex, AgentType::Gemini]
        .iter()
        .enumerate()
    {
        let agent_name = agent_type.to_string().to_lowercase();
        let auth_mode = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());

        let (count, _) = app
            .provider_quota
            .get(agent_type)
            .copied()
            .unwrap_or((0, Instant::now()));

        if auth_mode == "api" {
            spans.push(Span::styled(
                format!("{}: {} (API)", agent_type, count),
                Style::default().fg(Color::Cyan),
            ));
        } else {
            let max = match agent_type {
                AgentType::Claude => 50,
                AgentType::Codex => 60,
                AgentType::Gemini => 1000,
                _ => 100,
            };
            let ratio = count as f32 / max as f32;
            let color = if ratio > 0.8 {
                Color::Red
            } else if ratio > 0.5 {
                Color::Yellow
            } else {
                Color::Green
            };
            spans.push(Span::styled(
                format!("{}: {}/{} (5h)", agent_type, count, max),
                Style::default().fg(color),
            ));
        }

        if idx < 2 {
            spans.push(Span::styled(
                " \u{2502} ",
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    spans
}

fn render_task_board(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusArea::TaskBoard;

    // DX-038: Check for subscription risk
    let state_mgr_warn = StateManager::new(&app.forge_dir);
    let has_sub_risk = ["claude", "codex", "gemini"].iter().any(|agent| {
        state_mgr_warn
            .get_agent_auth(agent)
            .unwrap_or_else(|_| "subscription".to_string())
            == "subscription"
    });

    let border_style = if has_sub_risk {
        Style::default().fg(Color::Yellow)
    } else if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Phase-aware title with progress counts
    let (phase_done, phase_total) = phase_progress(app);
    let sub_suffix = if has_sub_risk { " \u{26A0} SUB" } else { "" };
    let title = if app.project_name.is_empty() {
        format!(
            " FORGE DASHBOARD \u{2014} {} ({}/{}){} ",
            app.phase, phase_done, phase_total, sub_suffix
        )
    } else {
        format!(
            " FORGE DASHBOARD \u{2014} {} \u{2014} {} ({}/{}){} ",
            app.project_name, app.phase, phase_done, phase_total, sub_suffix
        )
    };

    let header = Row::new(vec![
        Cell::from("ID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Agent").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Title").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    // Sort tasks hierarchically: parents first, then children indented
    let sorted_tasks = hierarchical_sort(&app.tasks);

    let rows: Vec<Row> = sorted_tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let (status_str, status_color) = match task.status {
                TaskStatus::Completed => ("\u{2713} Done", Color::Green),
                TaskStatus::Failed => ("\u{2717} Failed", Color::Red),
                TaskStatus::InProgress => ("\u{26A1} Running", Color::Yellow),
                TaskStatus::Blocked => ("\u{2297} Blocked", Color::Magenta),
                TaskStatus::Pending => ("\u{23F8} Pending", Color::White),
                TaskStatus::Assigned => ("\u{279C} Assigned", Color::Cyan),
            };

            let agent_str = task
                .assigned_to
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "-".into());

            let row_style = if focused && i == app.selected_index {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            // Indent subtask IDs (tasks with a parent)
            let id_display = if task.parent_task.is_some() {
                format!(" {}", task.id)
            } else {
                task.id.clone()
            };

            Row::new(vec![
                Cell::from(id_display),
                Cell::from(status_str).style(Style::default().fg(status_color)),
                Cell::from(agent_str),
                Cell::from(task.title.clone()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths)
        .header(header.style(Style::default().fg(Color::Cyan)))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        );

    f.render_widget(table, area);
}

fn render_agent_panes(f: &mut Frame, app: &App, area: Rect) {
    let rows =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let top_cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    let bot_cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);

    let pane_areas = [top_cols[0], top_cols[1], bot_cols[0], bot_cols[1]];

    for (idx, &pane_area) in pane_areas.iter().enumerate() {
        render_single_pane(f, app, idx, pane_area, false);
    }
}

fn render_single_pane(f: &mut Frame, app: &App, idx: usize, area: Rect, expanded: bool) {
    let focused = app.focus == FocusArea::Pane(idx);
    let border_style = if focused || expanded {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let label = pane_label(idx);

    // Pane 3: shell (when active) or summary
    if idx == 3 {
        if app.shell_active {
            render_shell_pane(f, app, area, focused || expanded);
        } else {
            render_summary_pane_inner(f, app, label, area, border_style);
        }
        return;
    }

    let agent = match pane_agent(idx) {
        Some(a) => a,
        None => return,
    };

    let running_task = app.agent_running_task.get(&agent);
    let title = match running_task {
        Some(tid) => format!(" {} [{}] ", label, tid),
        None => format!(" {} ", label),
    };

    let all_lines: Vec<&str> = app
        .agent_outputs
        .get(&agent)
        .map(|buf| buf.iter().map(|l| l.as_str()).collect())
        .unwrap_or_default();

    let inner_height = area.height.saturating_sub(2) as usize;
    let total = all_lines.len();
    let scroll = app.pane_scroll[idx];

    // Calculate visible window: show lines [start..end) from the buffer
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(inner_height);

    let mut visible_lines: Vec<Line> = all_lines[start..end]
        .iter()
        .map(|&l| Line::from(l))
        .collect();

    // Show scroll indicator if not at bottom
    if scroll > 0 && inner_height > 0 {
        let indicator = format!("[+{} lines below]", scroll);
        // Replace last visible line with indicator
        if !visible_lines.is_empty() {
            *visible_lines.last_mut().unwrap() = Line::from(Span::styled(
                indicator,
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    // Show backoff countdown if agent is rate-limited
    if let Some(backoff) = app.agent_backoff.get(&agent)
        && let Some(next_retry) = backoff.next_retry
    {
        let now = Instant::now();
        if now < next_retry {
            let remaining_secs = (next_retry - now).as_secs();
            let indicator = format!(
                "--- Rate limited. Retrying in {}s... (attempt {}/{}) ---",
                remaining_secs, backoff.attempt, MAX_BACKOFF_ATTEMPTS
            );
            visible_lines.push(Line::from(Span::styled(
                indicator,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    let paragraph = Paragraph::new(visible_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    );

    f.render_widget(paragraph, area);
}

fn render_shell_pane(f: &mut Frame, app: &App, area: Rect, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if active {
        " Shell (Esc:unfocus | Ctrl+D:close) "
    } else {
        " Shell (s:focus) "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible_height = inner.height as usize;
    let total = app.shell_output.len();
    let skip = total.saturating_sub(visible_height);
    let lines: Vec<Line> = app
        .shell_output
        .iter()
        .skip(skip)
        .map(|s| Line::from(s.as_str()))
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn render_summary_pane_inner(
    f: &mut Frame,
    app: &App,
    label: &str,
    area: Rect,
    border_style: Style,
) {
    let running = app.running_task_ids.len();
    let completed = app
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let failed = app
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();
    let total = app.tasks.len();
    let mode = if app.watch_mode { "Watch" } else { "Auto" };

    let phase_color = match app.phase {
        DashboardPhase::Build => Color::Yellow,
        DashboardPhase::Verify => Color::Cyan,
        DashboardPhase::Complete => Color::Green,
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Phase: ", Style::default().fg(phase_color)),
            Span::styled(
                format!("{}", app.phase),
                Style::default()
                    .fg(phase_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Running: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{}/{}", running, app.parallel_limit)),
        ]),
        Line::from(vec![
            Span::styled("Completed: ", Style::default().fg(Color::Green)),
            Span::raw(format!("{}/{}", completed, total)),
        ]),
        Line::from(vec![
            Span::styled("Failed: ", Style::default().fg(Color::Red)),
            Span::raw(format!("{}", failed)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Mode: ", Style::default().fg(Color::Cyan)),
            Span::raw(mode),
        ]),
        Line::from(vec![
            Span::styled("Parallel: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}", app.parallel_limit)),
        ]),
    ];

    // Show agents in backoff
    let backoff_count = app
        .agent_backoff
        .values()
        .filter(|b| b.next_retry.is_some())
        .count();
    if backoff_count > 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Backoff: ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{} agent(s)", backoff_count)),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", label))
            .border_style(border_style),
    );

    f.render_widget(paragraph, area);
}

/// Compute progress counts for the current dashboard phase.
fn phase_progress(app: &App) -> (usize, usize) {
    let phase_tasks: Vec<&Task> = match app.phase {
        DashboardPhase::Build => app
            .tasks
            .iter()
            .filter(|t| t.phase.is_none() || t.phase == Some(TaskPhase::Build))
            .collect(),
        DashboardPhase::Verify => app
            .tasks
            .iter()
            .filter(|t| t.phase == Some(TaskPhase::Verify) || t.phase == Some(TaskPhase::Fix))
            .collect(),
        DashboardPhase::Complete => app.tasks.iter().collect(),
    };

    let done = phase_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed)
        .count();
    (done, phase_tasks.len())
}

/// Sort tasks hierarchically: parents first, children immediately after their parent.
/// Children (tasks with parent_task set) are indented under their parent.
pub fn hierarchical_sort(tasks: &[Task]) -> Vec<Task> {
    use std::collections::HashMap;

    // Build a map of parent_id -> children
    let mut children_map: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut roots: Vec<&Task> = Vec::new();

    for task in tasks {
        if let Some(ref parent_id) = task.parent_task {
            children_map
                .entry(parent_id.as_str())
                .or_default()
                .push(task);
        } else {
            roots.push(task);
        }
    }

    // Sort children by ID within each group
    for children in children_map.values_mut() {
        children.sort_by(|a, b| a.id.cmp(&b.id));
    }

    let mut result = Vec::with_capacity(tasks.len());
    for root in &roots {
        result.push((*root).clone());
        if let Some(children) = children_map.get(root.id.as_str()) {
            for child in children {
                result.push((*child).clone());
            }
        }
    }

    // Add orphan children (whose parent isn't in the task list)
    for task in tasks {
        if task.parent_task.is_some() && !result.iter().any(|t| t.id == task.id) {
            result.push(task.clone());
        }
    }

    result
}

fn render_event_log(f: &mut Frame, app: &App, area: Rect) {
    let border_style = Style::default().fg(Color::DarkGray);

    let mut items: Vec<ListItem> = app
        .events
        .iter()
        .map(|e| ListItem::new(Line::from(e.as_str())))
        .collect();

    // DX-022: Append completion banner when all tasks are done
    if app.all_complete {
        let total = app.tasks.len();
        let completed = app
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        let elapsed = app.started_at.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;

        items.push(ListItem::new(Line::from("")));
        items.push(ListItem::new(Line::from(Span::styled(
            format!(
                "--- All {}/{} tasks completed in {}m {}s ---",
                completed, total, mins, secs
            ),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))));
        items.push(ListItem::new(Line::from(Span::styled(
            "Press q to exit | r retry | \u{2191}\u{2193} scroll | Tab switch panes",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let start = items.len().saturating_sub(inner_height);
    let visible: Vec<ListItem> = items.into_iter().skip(start).collect();

    let list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Event Log ")
            .border_style(border_style),
    );

    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::{AgentType, Task};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    #[test]
    fn test_render_does_not_panic() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        app.tasks = vec![
            Task::new("T-001", "Design API", "Design the REST API"),
            Task::new("T-002", "Implement auth", "Add authentication"),
        ];
        app.events.push_back("[12:00:00] Started T-001".to_string());

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("FORGE DASHBOARD"));
        assert!(content.contains("T-001"));
    }

    #[test]
    fn test_render_with_empty_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, true);

        terminal.draw(|f| render(f, &app)).unwrap();
    }

    #[test]
    fn test_render_completion_banner() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        let mut t1 = Task::new("T-001", "Task one", "done");
        t1.status = TaskStatus::Completed;
        let mut t2 = Task::new("T-002", "Task two", "done");
        t2.status = TaskStatus::Completed;
        app.tasks = vec![t1, t2];
        app.all_complete = true;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("All 2/2 tasks completed"));
        assert!(content.contains("Press q to exit"));
    }

    #[test]
    fn test_render_footer_visible() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("q:Quit"));
        assert!(content.contains("Tab:Focus"));
    }

    #[test]
    fn test_render_focused_pane_highlight() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.focus = FocusArea::Pane(0);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        // Claude pane should be visible
        assert!(content.contains("Claude"));
    }

    #[test]
    fn test_render_expanded_pane() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        app.expanded_pane = Some(0);
        let buf = app.agent_outputs.get_mut(&AgentType::Claude).unwrap();
        buf.push_back("expanded line test".to_string());

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        // Should show expanded pane content but NOT task board
        assert!(content.contains("expanded line test"));
        assert!(content.contains("Claude"));
        // Task Board should not be visible in expanded mode
        assert!(!content.contains("Task Board"));
    }

    #[test]
    fn test_render_scroll_indicator() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) =
            App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);
        let buf = app.agent_outputs.get_mut(&AgentType::Claude).unwrap();
        for i in 0..50 {
            buf.push_back(format!("line {}", i));
        }
        app.pane_scroll[0] = 5;
        app.pane_pinned[0] = true;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("+5 lines below"));
    }

    #[test]
    fn test_render_phase_in_title() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (app, _rx, _tx) = App::new(PathBuf::from("/tmp/test"), PathBuf::from("/tmp"), 3, false);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("BUILD"));
        assert!(content.contains("FORGE DASHBOARD"));
    }

    #[test]
    fn test_hierarchical_sort_parents_before_children() {
        use crate::core::task::TaskPhase;

        let mut parent = Task::new("T-001", "Parent task", "desc");
        parent.phase = None;

        let mut child = Task::new("V-001", "Verify: Parent task", "desc");
        child.parent_task = Some("T-001".to_string());
        child.phase = Some(TaskPhase::Verify);

        let mut other = Task::new("T-002", "Other task", "desc");
        other.phase = None;

        // Input order: child, other, parent (scrambled)
        let tasks = vec![child.clone(), other.clone(), parent.clone()];
        let sorted = hierarchical_sort(&tasks);

        assert_eq!(sorted.len(), 3);
        // T-001 should come before V-001 (parent before child)
        let parent_pos = sorted.iter().position(|t| t.id == "T-001").unwrap();
        let child_pos = sorted.iter().position(|t| t.id == "V-001").unwrap();
        assert!(parent_pos < child_pos, "parent should come before child");
    }

    #[test]
    fn test_hierarchical_sort_preserves_all_tasks() {
        let t1 = Task::new("T-001", "Task 1", "desc");
        let t2 = Task::new("T-002", "Task 2", "desc");
        let mut v1 = Task::new("V-001", "Verify 1", "desc");
        v1.parent_task = Some("T-001".to_string());

        let tasks = vec![t1, t2, v1];
        let sorted = hierarchical_sort(&tasks);
        assert_eq!(sorted.len(), 3);
    }

    // ── DX-037: Quota span tests ────────────────────────────────────

    #[test]
    fn test_quota_spans_api_mode() {
        // Create app with a forge_dir that has API mode set
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Write a state.json with claude auth = api
        let state_json = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "brain": { "provider": "rule-based" },
            "tools": [],
            "updated_at": "2026-01-01T00:00:00Z",
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "agent_auth": { "claude": "api", "codex": "api", "gemini": "api" },
            "agent_permissions": {},
            "active_locks": {},
            "scheduler": {
                "rotation": false,
                "pacing_min_secs": 64,
                "pacing_max_secs": 179
            },
            "git": { "auto_commit": false, "strategy": "branch-per-task" }
        });
        std::fs::write(forge_dir.join("state.json"), state_json.to_string()).unwrap();

        let (app, _rx, _tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);

        let spans = build_quota_spans(&app);
        // All three providers should show "(API)" with cyan styling
        let text: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("(API)"), "Expected API label, got: {}", text);
        // Should have 3 agent entries (lowercase display)
        assert!(text.contains("claude"));
        assert!(text.contains("codex"));
        assert!(text.contains("gemini"));
    }

    #[test]
    fn test_quota_spans_subscription_colors() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let state_json = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "brain": { "provider": "rule-based" },
            "tools": [],
            "updated_at": "2026-01-01T00:00:00Z",
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "agent_auth": {
                "claude": "subscription",
                "codex": "subscription",
                "gemini": "subscription"
            },
            "agent_permissions": {},
            "active_locks": {},
            "scheduler": {
                "rotation": false,
                "pacing_min_secs": 64,
                "pacing_max_secs": 179
            },
            "git": { "auto_commit": false, "strategy": "branch-per-task" }
        });
        std::fs::write(forge_dir.join("state.json"), state_json.to_string()).unwrap();

        let (mut app, _rx, _tx) = App::new(forge_dir, tmp.path().to_path_buf(), 3, false);

        // Set Claude to 45/50 (>80% = red), Codex to 35/60 (>50% = yellow), Gemini to 100/1000 (<50% = green)
        app.provider_quota
            .insert(AgentType::Claude, (45, Instant::now()));
        app.provider_quota
            .insert(AgentType::Codex, (35, Instant::now()));
        app.provider_quota
            .insert(AgentType::Gemini, (100, Instant::now()));

        let spans = build_quota_spans(&app);
        let text: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert!(
            text.contains("45/50"),
            "Expected Claude quota, got: {}",
            text
        );
        assert!(
            text.contains("35/60"),
            "Expected Codex quota, got: {}",
            text
        );
        assert!(
            text.contains("100/1000"),
            "Expected Gemini quota, got: {}",
            text
        );

        // Check colors via span styles
        let claude_span = &spans[0];
        assert_eq!(
            claude_span.style.fg,
            Some(Color::Red),
            "Claude at 90% should be red"
        );
        let codex_span = &spans[2]; // index 2 because index 1 is the separator
        assert_eq!(
            codex_span.style.fg,
            Some(Color::Yellow),
            "Codex at 58% should be yellow"
        );
        let gemini_span = &spans[4]; // index 4
        assert_eq!(
            gemini_span.style.fg,
            Some(Color::Green),
            "Gemini at 10% should be green"
        );
    }

    // ── DX-038: Subscription risk tests ─────────────────────────────

    #[test]
    fn test_subscription_risk_detected_claude() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Claude in subscription mode (default)
        let state_json = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "brain": { "provider": "rule-based" },
            "tools": [],
            "updated_at": "2026-01-01T00:00:00Z",
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "agent_auth": { "claude": "subscription" },
            "agent_permissions": {},
            "active_locks": {},
            "scheduler": {
                "rotation": false,
                "pacing_min_secs": 64,
                "pacing_max_secs": 179
            },
            "git": { "auto_commit": false, "strategy": "branch-per-task" }
        });
        std::fs::write(forge_dir.join("state.json"), state_json.to_string()).unwrap();

        let result = crate::cli::start::load_state_for_risk_check(&forge_dir);
        assert!(result.is_some(), "Should detect subscription risk");
        let warning = result.unwrap();
        assert!(warning.contains("SUBSCRIPTION RISK DETECTED"));
        assert!(warning.contains("claude.auth api"));
    }

    #[test]
    fn test_no_risk_with_api_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let state_json = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "brain": { "provider": "rule-based" },
            "tools": [],
            "updated_at": "2026-01-01T00:00:00Z",
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "agent_auth": { "claude": "api", "codex": "api", "gemini": "api" },
            "agent_permissions": {},
            "active_locks": {},
            "scheduler": {
                "rotation": false,
                "pacing_min_secs": 64,
                "pacing_max_secs": 179
            },
            "git": { "auto_commit": false, "strategy": "branch-per-task" }
        });
        std::fs::write(forge_dir.join("state.json"), state_json.to_string()).unwrap();

        let result = crate::cli::start::load_state_for_risk_check(&forge_dir);
        assert!(result.is_none(), "API mode should not trigger risk warning");
    }

    #[test]
    fn test_subscription_risk_codex_no_block() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Codex subscription, Claude API — should NOT block (only Claude sub blocks)
        let state_json = serde_json::json!({
            "version": "1.0.0",
            "project_name": "test",
            "created_at": "2026-01-01T00:00:00Z",
            "brain": { "provider": "rule-based" },
            "tools": [],
            "updated_at": "2026-01-01T00:00:00Z",
            "task_summary": { "total": 0, "pending": 0, "in_progress": 0, "completed": 0, "failed": 0, "blocked": 0 },
            "agent_auth": { "claude": "api", "codex": "subscription", "gemini": "api" },
            "agent_permissions": {},
            "active_locks": {},
            "scheduler": {
                "rotation": false,
                "pacing_min_secs": 64,
                "pacing_max_secs": 179
            },
            "git": { "auto_commit": false, "strategy": "branch-per-task" }
        });
        std::fs::write(forge_dir.join("state.json"), state_json.to_string()).unwrap();

        let result = crate::cli::start::load_state_for_risk_check(&forge_dir);
        assert!(
            result.is_none(),
            "Codex subscription should NOT block (only Claude)"
        );
    }
}
