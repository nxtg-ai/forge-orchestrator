use crossterm::event::{self, Event, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

pub enum TuiEvent {
    Key(KeyEvent),
    Tick,
    Resize(u16, u16),
}

/// Spawns a blocking task that polls crossterm for keyboard input every 100ms.
/// Sends Key events on key press, Tick events otherwise.
pub fn spawn_event_listener(tx: mpsc::UnboundedSender<TuiEvent>) {
    tokio::task::spawn_blocking(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) if tx.send(TuiEvent::Key(key)).is_err() => {
                        break;
                    }
                    Ok(Event::Resize(cols, rows))
                        if tx.send(TuiEvent::Resize(cols, rows)).is_err() =>
                    {
                        break;
                    }
                    _ => {}
                },
                Ok(false) => {
                    if tx.send(TuiEvent::Tick).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}
