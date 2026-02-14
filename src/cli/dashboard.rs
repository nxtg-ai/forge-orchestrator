use crate::core::event::EventLogger;
use crate::tui::app::App;
use crate::tui::event::{TuiEvent, spawn_event_listener};
use crate::tui::ui;
use crossterm::event::KeyEventKind;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use std::path::Path;

pub async fn execute(
    project_root: &Path,
    parallel_limit: usize,
    watch_mode: bool,
) -> anyhow::Result<()> {
    let forge_dir = project_root.join(".forge");

    if !forge_dir.exists() {
        anyhow::bail!("Forge is not initialized. Run `forge init` first.");
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

    // DX-021 Part 2: Reset any orphaned in-progress tasks from a previous run
    app.reset_orphaned_tasks()?;
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

    // Main loop — priority drain pattern (DX-026):
    // Keys are drained first to prevent starvation under heavy agent output.
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(50));
    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        // 1. PRIORITY: Drain ALL pending key/tick events first
        while let Ok(tui_event) = tui_rx.try_recv() {
            match tui_event {
                TuiEvent::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        app.handle_key(key, &agent_tx);
                        if app.should_quit {
                            break;
                        }
                    }
                }
                TuiEvent::Tick => {
                    app.handle_tick(&agent_tx)?;
                }
            }
        }
        if app.should_quit {
            break;
        }

        // 2. Then drain pending agent events (batch up to 50 per frame to stay responsive)
        let mut agent_count = 0;
        while let Ok(agent_event) = agent_rx.try_recv() {
            app.handle_agent_event(agent_event, &agent_tx)?;
            agent_count += 1;
            if agent_count >= 50 {
                break;
            }
        }

        // 3. If nothing was ready, wait for the next event with a short timeout
        //    This prevents busy-spinning when idle
        if agent_count == 0 {
            tokio::select! {
                Some(tui_event) = tui_rx.recv() => {
                    match tui_event {
                        TuiEvent::Key(key) => {
                            if key.kind == KeyEventKind::Press {
                                app.handle_key(key, &agent_tx);
                            }
                        }
                        TuiEvent::Tick => {
                            app.handle_tick(&agent_tx)?;
                        }
                    }
                }
                Some(agent_event) = agent_rx.recv() => {
                    app.handle_agent_event(agent_event, &agent_tx)?;
                }
                _ = interval.tick() => {
                    // Periodic tick to keep UI refreshed even when idle
                }
            }
        }
    }

    // DX-021 Part 1: Reset running tasks back to pending before exit
    app.cleanup_running_tasks();

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
