use crate::core::task::TaskStatus;
use crate::tui::app::{pane_agent, pane_label, App, FocusArea, MAX_BACKOFF_ATTEMPTS};
use ratatui::layout::{Constraint, Layout, Rect};
use std::time::Instant;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    // If a pane is expanded, render only that pane full-screen + footer
    if let Some(idx) = app.expanded_pane {
        let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
            .split(f.area());
        render_single_pane(f, app, idx, chunks[0], true);
        render_footer(f, app, chunks[1]);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Percentage(30), // Task board
        Constraint::Percentage(50), // Agent panes 2x2
        Constraint::Percentage(20), // Event log
        Constraint::Length(1),      // Footer
    ])
    .split(f.area());

    render_task_board(f, app, chunks[0]);
    render_agent_panes(f, app, chunks[1]);
    render_event_log(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = if app.all_complete {
        "q:Quit | \u{2191}\u{2193}:Scroll | Tab:Switch Pane"
    } else if app.expanded_pane.is_some() {
        "Esc:Back | \u{2191}\u{2193}:Scroll | Home/End:Jump | q:Quit"
    } else {
        "q:Quit | Tab:Focus | \u{2191}\u{2193}:Navigate | Enter/f:Expand | r:Retry"
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(paragraph, area);
}

fn render_task_board(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusArea::TaskBoard;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let header = Row::new(vec![
        Cell::from("ID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Agent").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Title").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = app
        .tasks
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

            Row::new(vec![
                Cell::from(task.id.clone()),
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
                .title(" Task Board ")
                .border_style(border_style),
        );

    f.render_widget(table, area);
}

fn render_agent_panes(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
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

    // Summary pane (index 3) has special rendering
    if idx == 3 {
        render_summary_pane_inner(f, app, label, area, border_style);
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

    let mut lines = vec![
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
    let backoff_count = app.agent_backoff.values().filter(|b| b.next_retry.is_some()).count();
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
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    #[test]
    fn test_render_does_not_panic() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );

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
        assert!(content.contains("Task Board"));
        assert!(content.contains("T-001"));
    }

    #[test]
    fn test_render_with_empty_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            true,
        );

        terminal.draw(|f| render(f, &app)).unwrap();
    }

    #[test]
    fn test_render_completion_banner() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );

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

        let (app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );

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

        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
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

        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
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

        let (mut app, _rx, _tx) = App::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp"),
            3,
            false,
        );
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
}
