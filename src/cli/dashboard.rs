use crate::core::event::EventLogger;
use crate::tui::app::App;
use crate::tui::event::{spawn_event_listener, TuiEvent};
use crate::tui::ui;
use crossterm::event::KeyEventKind;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::path::Path;

pub async fn execute(project_root: &Path, parallel_limit: usize, watch_mode: bool) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        anyhow::bail!(
            "Forge is not initialized. Run `forge init` first."
        );
    }

    // Install a panic hook that restores the terminal before printing the panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Enter raw mode + alternate screen
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (mut app, mut agent_rx, agent_tx) = App::new(
        forge_dir.clone(),
        project_root.to_path_buf(),
        parallel_limit,
        watch_mode,
    );

    // Load initial tasks
    app.reload_tasks()?;

    // Load recent events from disk
    let event_logger = EventLogger::new(&forge_dir);
    if let Ok(recent) = event_logger.read_recent(20) {
        for ev in recent {
            let ts = ev.timestamp.format("%H:%M:%S");
            app.events.push_back(format!("[{}] {}", ts, ev.message));
        }
    }

    // Schedule initial tasks (if not watch mode)
    app.schedule_unblocked_tasks(&agent_tx);

    // Start keyboard event listener
    let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel();
    spawn_event_listener(tui_tx);

    // Main loop
    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        tokio::select! {
            Some(tui_event) = tui_rx.recv() => {
                match tui_event {
                    TuiEvent::Key(key) => {
                        // crossterm 0.28 fires Press + Release on some platforms
                        if key.kind == KeyEventKind::Press {
                            app.handle_key(key);
                            if app.should_quit {
                                break;
                            }
                        }
                    }
                    TuiEvent::Tick => {
                        app.reload_tasks()?;
                        if !app.watch_mode && app.is_all_done() && app.running_task_ids.is_empty() {
                            break;
                        }
                    }
                }
            }
            Some(agent_event) = agent_rx.recv() => {
                app.handle_agent_event(agent_event, &agent_tx)?;
            }
        }
    }

    // Cleanup: restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Print summary
    let completed = app
        .tasks
        .iter()
        .filter(|t| t.status == crate::core::task::TaskStatus::Completed)
        .count();
    let failed = app
        .tasks
        .iter()
        .filter(|t| t.status == crate::core::task::TaskStatus::Failed)
        .count();

    println!(
        "Dashboard complete. {} completed, {} failed.",
        completed, failed
    );

    Ok(())
}
