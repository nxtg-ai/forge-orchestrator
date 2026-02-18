use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::style::{Color, Modifier, Style};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::core::task::AgentType;
use crate::tui::app::AgentEvent;

/// A styled span of text with associated terminal style.
#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub style: Style,
}

/// A line composed of styled spans, produced by parsing ANSI sequences.
#[derive(Debug, Clone)]
pub struct StyledLine {
    pub spans: Vec<StyledSpan>,
}

impl StyledLine {
    pub fn to_ratatui_line(&self) -> ratatui::text::Line<'static> {
        let spans: Vec<ratatui::text::Span<'static>> = self
            .spans
            .iter()
            .map(|s| ratatui::text::Span::styled(s.text.clone(), s.style))
            .collect();
        ratatui::text::Line::from(spans)
    }
}

/// Convert a vt100 cell color to a ratatui color.
fn convert_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(0) => Some(Color::Black),
        vt100::Color::Idx(1) => Some(Color::Red),
        vt100::Color::Idx(2) => Some(Color::Green),
        vt100::Color::Idx(3) => Some(Color::Yellow),
        vt100::Color::Idx(4) => Some(Color::Blue),
        vt100::Color::Idx(5) => Some(Color::Magenta),
        vt100::Color::Idx(6) => Some(Color::Cyan),
        vt100::Color::Idx(7) => Some(Color::White),
        vt100::Color::Idx(n) => Some(Color::Indexed(n)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

/// Build a ratatui Style from a vt100 cell's attributes.
fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    if let Some(fg) = convert_color(cell.fgcolor()) {
        style = style.fg(fg);
    }
    if let Some(bg) = convert_color(cell.bgcolor()) {
        style = style.bg(bg);
    }
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// Convert a vt100 screen into styled lines for rendering.
/// Groups consecutive cells with identical styles into spans.
/// Trims trailing whitespace from each row.
fn screen_to_styled_lines(screen: &vt100::Screen) -> Vec<StyledLine> {
    let (rows, cols) = screen.size();
    let mut lines = Vec::with_capacity(rows as usize);

    for row in 0..rows {
        let mut spans: Vec<StyledSpan> = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();

        for col in 0..cols {
            let cell = screen.cell(row, col);
            // Skip wide-char continuation cells
            if let Some(cell) = cell {
                if cell.is_wide_continuation() {
                    continue;
                }
                let style = cell_style(cell);
                let text = cell.contents();

                if style == current_style {
                    current_text.push_str(&text);
                } else {
                    if !current_text.is_empty() {
                        spans.push(StyledSpan {
                            text: std::mem::take(&mut current_text),
                            style: current_style,
                        });
                    }
                    current_style = style;
                    current_text.push_str(&text);
                }
            }
        }

        // Flush remaining text
        if !current_text.is_empty() {
            spans.push(StyledSpan {
                text: current_text,
                style: current_style,
            });
        }

        // Trim trailing whitespace from last span(s) with default style
        while let Some(last) = spans.last_mut() {
            if last.style == Style::default() {
                let trimmed = last.text.trim_end().to_string();
                if trimmed.is_empty() {
                    spans.pop();
                } else {
                    last.text = trimmed;
                    break;
                }
            } else {
                break;
            }
        }

        lines.push(StyledLine { spans });
    }

    lines
}

/// PTY session lifecycle manager.
/// Uses vt100 crate for full virtual terminal emulation — handles cursor
/// positioning, scroll regions, alternate screen, and all ANSI sequences
/// that TUI apps (Claude, Codex, Gemini) rely on.
pub struct PtySession {
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    parser: Arc<Mutex<vt100::Parser>>,
    input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
}

impl PtySession {
    /// Spawn a new PTY session with the given command.
    pub fn spawn(
        cmd: CommandBuilder,
        size: PtySize,
        agent_tx: mpsc::UnboundedSender<AgentEvent>,
        task_id: String,
        agent: AgentType,
        _max_lines: usize,
    ) -> anyhow::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(size)?;

        let child = pair.slave.spawn_command(cmd)?;
        // Drop the slave -- the child owns it now
        drop(pair.slave);

        let child = Arc::new(Mutex::new(child));

        // vt100 parser with same dimensions as the PTY + scrollback buffer
        let parser = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, 500)));

        // Reader thread: reads from PTY master, feeds into vt100 parser.
        // When the process exits (EOF), waits for exit code and sends Completed.
        let mut reader = pair.master.try_clone_reader()?;
        let parser_clone = Arc::clone(&parser);
        let child_clone = Arc::clone(&child);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF — process exited
                    Ok(n) => {
                        if let Ok(mut guard) = parser_clone.lock() {
                            guard.process(&buf[..n]);
                        }
                        // Send an empty sentinel to wake the event loop for redraw
                        let _ = agent_tx.send(AgentEvent::Output {
                            task_id: task_id.clone(),
                            agent: agent.clone(),
                            line: String::new(),
                        });
                    }
                    Err(_) => break,
                }
            }
            // Process exited — get exit code and notify event loop
            let (success, exit_code) = if let Ok(mut guard) = child_clone.lock() {
                match guard.wait() {
                    Ok(status) => (status.success(), if status.success() { 0 } else { 1 }),
                    Err(_) => (false, -1),
                }
            } else {
                (false, -1)
            };
            let _ = agent_tx.send(AgentEvent::Completed {
                task_id,
                agent,
                success,
                exit_code,
            });
        });

        // Writer thread: receives bytes from input channel, writes to PTY
        let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let mut writer = pair.master.take_writer()?;
        std::thread::spawn(move || {
            use std::io::Write;
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
        });

        Ok(Self {
            child,
            parser,
            input_tx,
            master: pair.master,
        })
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, bytes: &[u8]) {
        let _ = self.input_tx.send(bytes.to_vec());
    }

    /// Resize the PTY and the virtual terminal.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        // Keep vt100 parser in sync with PTY dimensions
        if let Ok(mut guard) = self.parser.lock() {
            guard.set_size(rows, cols);
        }
        Ok(())
    }

    /// Get a snapshot of the styled output lines from the virtual terminal screen.
    pub fn snapshot(&self) -> Vec<StyledLine> {
        self.parser
            .lock()
            .map(|guard| screen_to_styled_lines(guard.screen()))
            .unwrap_or_default()
    }

    /// Get total line count (screen rows).
    pub fn line_count(&self) -> usize {
        self.parser
            .lock()
            .map(|guard| guard.screen().size().0 as usize)
            .unwrap_or(0)
    }

    /// Try to check if the child has exited.
    pub fn try_wait(&mut self) -> Option<portable_pty::ExitStatus> {
        if let Ok(mut guard) = self.child.lock() {
            guard.try_wait().ok().flatten()
        } else {
            None
        }
    }

    /// Kill the child process.
    pub fn kill(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            let _ = guard.kill();
        }
    }

    /// Check if a pattern appears in the last N lines of PTY output.
    pub fn has_pattern_in_last_n(&self, pattern: &str, n: usize) -> bool {
        if let Ok(guard) = self.parser.lock() {
            let screen = guard.screen();
            let rows = screen.size().0 as usize;
            let start = rows.saturating_sub(n);
            for row in start..rows {
                let row_text = screen.contents_between(row as u16, 0, row as u16, screen.size().1);
                if row_text.contains(pattern) {
                    return true;
                }
            }
            false
        } else {
            false
        }
    }

    /// Schedule text to be written to the PTY after a fixed delay.
    /// Fallback when no ready_pattern is available.
    pub fn schedule_input(&self, text: String, delay_ms: u64) {
        let sender = self.input_tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            let _ = sender.send(text.into_bytes());
        });
    }

    /// Schedule text to be written when a pattern appears in the PTY output.
    /// Polls the virtual terminal screen every 200ms for the pattern.
    /// Falls back to timeout. This is the adaptive "golden egg" — no guessing
    /// delays, instant response when the agent's TUI is ready.
    pub fn schedule_input_when_ready(&self, text: String, pattern: String, timeout_ms: u64) {
        let sender = self.input_tx.clone();
        let parser = Arc::clone(&self.parser);
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_millis(timeout_ms);
            let poll_interval = std::time::Duration::from_millis(200);

            loop {
                // Check if pattern appears anywhere on the terminal screen
                if let Ok(guard) = parser.lock()
                    && guard.screen().contents().contains(&pattern)
                {
                    // Small grace period for TUI to finish rendering
                    drop(guard);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    // Send text and Enter separately — some TUIs need Enter
                    // as a distinct write to trigger submit
                    let bytes = text.into_bytes();
                    let (body, has_cr) = if bytes.ends_with(b"\r") {
                        (&bytes[..bytes.len() - 1], true)
                    } else {
                        (&bytes[..], false)
                    };
                    let _ = sender.send(body.to_vec());
                    if has_cr {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let _ = sender.send(b"\r".to_vec());
                    }
                    return;
                }

                // Check timeout — use same split logic as pattern-match path
                if start.elapsed() >= timeout {
                    let bytes = text.into_bytes();
                    let (body, has_cr) = if bytes.ends_with(b"\r") {
                        (&bytes[..bytes.len() - 1], true)
                    } else {
                        (&bytes[..], false)
                    };
                    let _ = sender.send(body.to_vec());
                    if has_cr {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let _ = sender.send(b"\r".to_vec());
                    }
                    return;
                }

                std::thread::sleep(poll_interval);
            }
        });
    }
}

/// Convert a crossterm KeyEvent to raw terminal bytes for PTY input.
pub fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char(c) if ctrl => {
            // Ctrl+A = 0x01, Ctrl+B = 0x02, ..., Ctrl+Z = 0x1a
            let code = (c.to_ascii_lowercase() as u8)
                .wrapping_sub(b'a')
                .wrapping_add(1);
            vec![code]
        }
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(1) => b"\x1bOP".to_vec(),
        KeyCode::F(2) => b"\x1bOQ".to_vec(),
        KeyCode::F(3) => b"\x1bOR".to_vec(),
        KeyCode::F(4) => b"\x1bOS".to_vec(),
        KeyCode::F(5) => b"\x1b[15~".to_vec(),
        KeyCode::F(6) => b"\x1b[17~".to_vec(),
        KeyCode::F(7) => b"\x1b[18~".to_vec(),
        KeyCode::F(8) => b"\x1b[19~".to_vec(),
        KeyCode::F(9) => b"\x1b[20~".to_vec(),
        KeyCode::F(10) => b"\x1b[21~".to_vec(),
        KeyCode::F(11) => b"\x1b[23~".to_vec(),
        KeyCode::F(12) => b"\x1b[24~".to_vec(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a vt100 parser, feed bytes, return styled lines.
    fn parse_to_lines(input: &[u8], rows: u16, cols: u16) -> Vec<StyledLine> {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(input);
        screen_to_styled_lines(parser.screen())
    }

    /// Extract all text from a StyledLine.
    fn line_text(line: &StyledLine) -> String {
        line.spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn test_plain_text_line() {
        let lines = parse_to_lines(b"hello\n", 24, 80);
        // First row should contain "hello"
        assert_eq!(line_text(&lines[0]), "hello");
    }

    #[test]
    fn test_ansi_red_text() {
        let lines = parse_to_lines(b"\x1b[31mERROR\x1b[0m\n", 24, 80);
        let error_span = lines[0]
            .spans
            .iter()
            .find(|s| s.text.contains("ERROR"))
            .expect("Should have ERROR span");
        assert_eq!(error_span.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_256_color() {
        let lines = parse_to_lines(b"\x1b[38;5;208mORANGE\x1b[0m\n", 24, 80);
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.text.contains("ORANGE"))
            .expect("Should have ORANGE span");
        assert_eq!(span.style.fg, Some(Color::Indexed(208)));
    }

    #[test]
    fn test_rgb_color() {
        let lines = parse_to_lines(b"\x1b[38;2;255;128;0mRGB\x1b[0m\n", 24, 80);
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.text.contains("RGB"))
            .expect("Should have RGB span");
        assert_eq!(span.style.fg, Some(Color::Rgb(255, 128, 0)));
    }

    #[test]
    fn test_bold_and_color() {
        let lines = parse_to_lines(b"\x1b[1;32mBOLD GREEN\x1b[0m\n", 24, 80);
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.text.contains("BOLD GREEN"))
            .expect("Should have BOLD GREEN span");
        assert_eq!(span.style.fg, Some(Color::Green));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_cursor_positioning() {
        // vt100 handles ESC[row;colH — our old parser ignored this
        let lines = parse_to_lines(b"\x1b[5;10Hhello", 24, 80);
        // Row 4 (0-indexed), col 9 should have "hello"
        let text = line_text(&lines[4]);
        assert!(
            text.contains("hello"),
            "Expected 'hello' at row 5, got: {:?}",
            text
        );
    }

    #[test]
    fn test_screen_clear() {
        // Write text, then clear screen — old parser would keep old text
        let lines = parse_to_lines(b"old text\x1b[2J\x1b[Hnew text", 24, 80);
        let all_text: String = lines
            .iter()
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("");
        assert!(!all_text.contains("old text"), "Screen should be cleared");
        assert!(all_text.contains("new text"), "New text should be visible");
    }

    #[test]
    fn test_carriage_return_overwrite() {
        let lines = parse_to_lines(b"50%\r100%\n", 24, 80);
        let text = line_text(&lines[0]);
        assert!(text.contains("100%"), "CR should overwrite: got {:?}", text);
        assert!(
            !text.contains("50%"),
            "Old text should be overwritten: got {:?}",
            text
        );
    }

    #[test]
    fn test_snapshot_returns_screen_rows() {
        let lines = parse_to_lines(b"hello\n", 10, 40);
        assert_eq!(lines.len(), 10, "Should return exactly screen height lines");
    }

    #[test]
    fn test_pattern_detection() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"What can I help you with?\n> ");
        assert!(parser.screen().contents().contains("help you with"));
    }

    #[test]
    fn test_key_to_bytes_char() {
        let key = KeyEvent::from(KeyCode::Char('a'));
        assert_eq!(key_event_to_bytes(&key), vec![0x61]);
    }

    #[test]
    fn test_key_to_bytes_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(&key), vec![0x03]);
    }

    #[test]
    fn test_key_to_bytes_arrows() {
        let up = KeyEvent::from(KeyCode::Up);
        assert_eq!(key_event_to_bytes(&up), b"\x1b[A".to_vec());

        let down = KeyEvent::from(KeyCode::Down);
        assert_eq!(key_event_to_bytes(&down), b"\x1b[B".to_vec());

        let right = KeyEvent::from(KeyCode::Right);
        assert_eq!(key_event_to_bytes(&right), b"\x1b[C".to_vec());

        let left = KeyEvent::from(KeyCode::Left);
        assert_eq!(key_event_to_bytes(&left), b"\x1b[D".to_vec());
    }

    #[test]
    fn test_has_pattern_in_last_n_found() {
        // Use a small screen so 4 lines of text land in the "last 5" rows
        let mut parser = vt100::Parser::new(6, 80, 0);
        parser.process(b"line 1\nline 2\nhello world\nline 4\n");
        let screen = parser.screen();
        let rows = screen.size().0 as usize;
        let start = rows.saturating_sub(5);
        let found = (start..rows).any(|row| {
            let text = screen.contents_between(row as u16, 0, row as u16, screen.size().1);
            text.contains("hello world")
        });
        assert!(found, "Pattern should be found in last 5 lines");
    }

    #[test]
    fn test_has_pattern_in_last_n_not_found() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"line 1\nline 2\nline 3\nline 4\nline 5\n");
        let screen = parser.screen();
        let rows = screen.size().0 as usize;
        let start = rows.saturating_sub(5);
        let found = (start..rows).any(|row| {
            let text = screen.contents_between(row as u16, 0, row as u16, screen.size().1);
            text.contains("hello world")
        });
        assert!(!found, "Pattern should NOT be found");
    }

    #[test]
    fn test_pty_echo() {
        use crate::tui::app::AgentEvent;
        use portable_pty::{CommandBuilder, PtySize};

        let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let mut cmd = CommandBuilder::new("bash");
        cmd.args(["-c", "echo 'hello from pty'; sleep 0.2"]);

        let mut session = PtySession::spawn(
            cmd,
            size,
            tx,
            "__test_echo__".to_string(),
            AgentType::Claude,
            1000,
        )
        .expect("Failed to spawn PTY echo");

        // Wait for process exit + reader thread to drain output
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if session.try_wait().is_some() {
                std::thread::sleep(std::time::Duration::from_millis(200));
                break;
            }
        }

        let lines = session.snapshot();
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.text.as_str()))
            .collect::<Vec<_>>()
            .join("");

        assert!(
            text.contains("hello from pty"),
            "Expected 'hello from pty' in output, got: {:?}",
            text
        );
    }

    #[test]
    fn test_pty_resize() {
        use crate::tui::app::AgentEvent;
        use portable_pty::{CommandBuilder, PtySize};

        let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let mut cmd = CommandBuilder::new("sleep");
        cmd.arg("1");

        let mut session = PtySession::spawn(
            cmd,
            size,
            tx,
            "__test_resize__".to_string(),
            AgentType::Claude,
            1000,
        )
        .expect("Failed to spawn PTY sleep");

        let result = session.resize(100, 40);
        assert!(result.is_ok(), "Resize should succeed");

        // Verify vt100 parser also resized
        let lines = session.snapshot();
        assert_eq!(lines.len(), 40, "After resize, should have 40 rows");

        let _ = session.kill();
    }

    #[test]
    fn test_styled_line_to_ratatui() {
        let line = StyledLine {
            spans: vec![
                StyledSpan {
                    text: "hello ".to_string(),
                    style: Style::default(),
                },
                StyledSpan {
                    text: "world".to_string(),
                    style: Style::default().fg(Color::Red),
                },
            ],
        };
        let rline = line.to_ratatui_line();
        assert_eq!(rline.spans.len(), 2);
        assert_eq!(rline.spans[0].content, "hello ");
        assert_eq!(rline.spans[1].content, "world");
    }
}
