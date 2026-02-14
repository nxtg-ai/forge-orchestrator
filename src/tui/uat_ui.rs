use crate::tui::uat_app::{UatApp, UatStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn render(f: &mut Frame, app: &UatApp) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
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
                UatStatus::HasFindings => {
                    Span::styled("  \u{2717} ", Style::default().fg(Color::Red))
                }
                UatStatus::Untested => {
                    Span::styled("  \u{25CB} ", Style::default().fg(Color::DarkGray))
                }
            };

            let title = truncate_chars(&t.task.title, 30);

            let id_style = if i == app.selected_task {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
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
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
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
                    Span::styled(
                        format!("  {} ", finding.id),
                        Style::default().fg(Color::Cyan),
                    ),
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

    let block = Block::default().borders(Borders::ALL).style(style);
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
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
