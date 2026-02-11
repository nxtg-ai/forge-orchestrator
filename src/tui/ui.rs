use crate::core::task::{AgentType, TaskStatus};
use crate::tui::app::{App, FocusArea};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Percentage(30),
        Constraint::Percentage(50),
        Constraint::Percentage(20),
    ])
    .split(f.area());

    render_task_board(f, app, chunks[0]);
    render_agent_panes(f, app, chunks[1]);
    render_event_log(f, app, chunks[2]);
}

fn render_task_board(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let focused = app.focus_area == FocusArea::TaskBoard;
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

fn render_agent_panes(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let focused = app.focus_area == FocusArea::AgentPanes;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let rows = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let top_cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    let bot_cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);

    render_agent_pane(f, app, &AgentType::Claude, "Claude", top_cols[0], border_style);
    render_agent_pane(f, app, &AgentType::Codex, "Codex", top_cols[1], border_style);
    render_agent_pane(f, app, &AgentType::Gemini, "Gemini", bot_cols[0], border_style);
    render_summary_pane(f, app, bot_cols[1], border_style);
}

fn render_agent_pane(
    f: &mut Frame,
    app: &App,
    agent: &AgentType,
    name: &str,
    area: ratatui::layout::Rect,
    border_style: Style,
) {
    let running_task = app.agent_running_task.get(agent);
    let title = match running_task {
        Some(tid) => format!(" {} [{}] ", name, tid),
        None => format!(" {} ", name),
    };

    let lines: Vec<Line> = app
        .agent_outputs
        .get(agent)
        .map(|buf| buf.iter().map(|l| Line::from(l.as_str())).collect())
        .unwrap_or_default();

    // Auto-scroll: only show the last N lines that fit
    let inner_height = area.height.saturating_sub(2) as usize; // -2 for borders
    let start = lines.len().saturating_sub(inner_height);
    let visible: Vec<Line> = lines.into_iter().skip(start).collect();

    let paragraph = Paragraph::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    );

    f.render_widget(paragraph, area);
}

fn render_summary_pane(
    f: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
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

    let lines = vec![
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

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Summary ")
            .border_style(border_style),
    );

    f.render_widget(paragraph, area);
}

fn render_event_log(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let focused = app.focus_area == FocusArea::EventLog;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let items: Vec<ListItem> = app
        .events
        .iter()
        .map(|e| ListItem::new(Line::from(e.as_str())))
        .collect();

    // Auto-scroll: show only the last items that fit
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
    use crate::core::task::Task;
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
        // Verify the buffer has content (not all spaces)
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
        // Just verifying no panic occurs
    }
}
