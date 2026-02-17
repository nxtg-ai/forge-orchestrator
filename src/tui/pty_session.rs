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

/// VTE-based ANSI escape sequence parser that collects styled lines.
pub struct AnsiLineCollector {
    lines: Vec<StyledLine>,
    current_line: Vec<StyledSpan>,
    current_text: String,
    current_style: Style,
    max_lines: usize,
    cr_pending: bool,
}

impl AnsiLineCollector {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_line: Vec::new(),
            current_text: String::new(),
            current_style: Style::default(),
            max_lines,
            cr_pending: false,
        }
    }

    /// Take a snapshot of the current styled lines (including any partial current line).
    pub fn snapshot(&self) -> Vec<StyledLine> {
        let mut result = self.lines.clone();
        // Include the current partial line
        let mut partial = self.current_line.clone();
        if !self.current_text.is_empty() {
            partial.push(StyledSpan {
                text: self.current_text.clone(),
                style: self.current_style,
            });
        }
        if !partial.is_empty() {
            result.push(StyledLine { spans: partial });
        }
        result
    }

    /// Total line count including partial current line.
    pub fn line_count(&self) -> usize {
        let has_partial = !self.current_line.is_empty() || !self.current_text.is_empty();
        self.lines.len() + if has_partial { 1 } else { 0 }
    }

    /// Flush current text into a span and complete the current line.
    fn complete_line(&mut self) {
        self.cr_pending = false; // \r\n → line is completed, not overwritten
        if !self.current_text.is_empty() {
            self.current_line.push(StyledSpan {
                text: std::mem::take(&mut self.current_text),
                style: self.current_style,
            });
        }
        let line = StyledLine {
            spans: std::mem::take(&mut self.current_line),
        };
        self.lines.push(line);
        // Ring buffer cap
        while self.lines.len() > self.max_lines {
            self.lines.remove(0);
        }
    }

    /// Carriage return: mark pending overwrite (deferred until next print).
    /// This handles `\r\n` (line ending) correctly — `\r` sets the flag,
    /// and `\n` completes the line without clearing it. Only if new text
    /// is printed after `\r` does the current line get cleared (for progress bars).
    fn carriage_return(&mut self) {
        self.cr_pending = true;
    }

    /// Apply the deferred carriage return: clear line for overwrite.
    fn apply_cr(&mut self) {
        if self.cr_pending {
            self.cr_pending = false;
            self.current_line.clear();
            self.current_text.clear();
        }
    }

    /// Flush current text segment before style change.
    fn flush_text(&mut self) {
        if !self.current_text.is_empty() {
            self.current_line.push(StyledSpan {
                text: std::mem::take(&mut self.current_text),
                style: self.current_style,
            });
        }
    }

    /// Apply SGR (Select Graphic Rendition) parameters.
    fn apply_sgr(&mut self, params: &[u16]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.current_style = Style::default(),
                1 => self.current_style = self.current_style.add_modifier(Modifier::BOLD),
                2 => self.current_style = self.current_style.add_modifier(Modifier::DIM),
                3 => self.current_style = self.current_style.add_modifier(Modifier::ITALIC),
                4 => self.current_style = self.current_style.add_modifier(Modifier::UNDERLINED),
                7 => self.current_style = self.current_style.add_modifier(Modifier::REVERSED),
                9 => self.current_style = self.current_style.add_modifier(Modifier::CROSSED_OUT),
                22 => {
                    self.current_style = self
                        .current_style
                        .remove_modifier(Modifier::BOLD)
                        .remove_modifier(Modifier::DIM)
                }
                23 => self.current_style = self.current_style.remove_modifier(Modifier::ITALIC),
                24 => self.current_style = self.current_style.remove_modifier(Modifier::UNDERLINED),
                27 => self.current_style = self.current_style.remove_modifier(Modifier::REVERSED),
                29 => {
                    self.current_style = self.current_style.remove_modifier(Modifier::CROSSED_OUT)
                }
                // Standard foreground colors
                30 => self.current_style = self.current_style.fg(Color::Black),
                31 => self.current_style = self.current_style.fg(Color::Red),
                32 => self.current_style = self.current_style.fg(Color::Green),
                33 => self.current_style = self.current_style.fg(Color::Yellow),
                34 => self.current_style = self.current_style.fg(Color::Blue),
                35 => self.current_style = self.current_style.fg(Color::Magenta),
                36 => self.current_style = self.current_style.fg(Color::Cyan),
                37 => self.current_style = self.current_style.fg(Color::White),
                // Extended foreground: 38;5;N or 38;2;R;G;B
                38 => {
                    if i + 1 < params.len() {
                        match params[i + 1] {
                            5 if i + 2 < params.len() => {
                                self.current_style =
                                    self.current_style.fg(Color::Indexed(params[i + 2] as u8));
                                i += 2;
                            }
                            2 if i + 4 < params.len() => {
                                self.current_style = self.current_style.fg(Color::Rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ));
                                i += 4;
                            }
                            _ => {}
                        }
                    }
                }
                39 => self.current_style = self.current_style.fg(Color::Reset),
                // Standard background colors
                40 => self.current_style = self.current_style.bg(Color::Black),
                41 => self.current_style = self.current_style.bg(Color::Red),
                42 => self.current_style = self.current_style.bg(Color::Green),
                43 => self.current_style = self.current_style.bg(Color::Yellow),
                44 => self.current_style = self.current_style.bg(Color::Blue),
                45 => self.current_style = self.current_style.bg(Color::Magenta),
                46 => self.current_style = self.current_style.bg(Color::Cyan),
                47 => self.current_style = self.current_style.bg(Color::White),
                // Extended background: 48;5;N or 48;2;R;G;B
                48 => {
                    if i + 1 < params.len() {
                        match params[i + 1] {
                            5 if i + 2 < params.len() => {
                                self.current_style =
                                    self.current_style.bg(Color::Indexed(params[i + 2] as u8));
                                i += 2;
                            }
                            2 if i + 4 < params.len() => {
                                self.current_style = self.current_style.bg(Color::Rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ));
                                i += 4;
                            }
                            _ => {}
                        }
                    }
                }
                49 => self.current_style = self.current_style.bg(Color::Reset),
                _ => {}
            }
            i += 1;
        }
    }
}

/// Implement the VTE Perform trait to handle escape sequences properly.
impl vte::Perform for AnsiLineCollector {
    fn print(&mut self, c: char) {
        self.apply_cr();
        self.current_text.push(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // BS -> cursor back (erase last char from current text)
            0x08 => {
                self.apply_cr();
                self.current_text.pop();
            }
            // LF / VT / FF -> complete line
            0x0a..=0x0c => {
                self.complete_line();
            }
            // CR -> carriage return
            0x0d => {
                self.carriage_return();
            }
            // TAB -> expand to spaces
            0x09 => {
                self.current_text.push_str("    ");
            }
            // BEL, etc -- ignore
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        match action {
            // SGR -- Select Graphic Rendition
            'm' => {
                self.flush_text();
                let param_vec: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();
                if param_vec.is_empty() {
                    self.apply_sgr(&[0]); // bare ESC[m = reset
                } else {
                    self.apply_sgr(&param_vec);
                }
            }
            // Erase in Line
            'K' => {
                let param = params
                    .iter()
                    .next()
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0);
                if param == 2 {
                    // Clear entire line
                    self.current_line.clear();
                    self.current_text.clear();
                }
                // param 0 (clear to end) is a no-op for our line-based model
            }
            // Ignore cursor movement, scrolling, etc. (MVP scope)
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
}

/// Feed bytes through the VTE parser into the collector.
pub fn feed_through_vte(parser: &mut vte::Parser, collector: &mut AnsiLineCollector, bytes: &[u8]) {
    parser.advance(collector, bytes);
}

/// PTY session lifecycle manager.
pub struct PtySession {
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    collector: Arc<Mutex<(AnsiLineCollector, vte::Parser)>>,
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
        max_lines: usize,
    ) -> anyhow::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(size)?;

        let child = pair.slave.spawn_command(cmd)?;
        // Drop the slave -- the child owns it now
        drop(pair.slave);

        let child = Arc::new(Mutex::new(child));

        let collector = Arc::new(Mutex::new((
            AnsiLineCollector::new(max_lines),
            vte::Parser::new(),
        )));

        // Reader thread: reads from PTY master, feeds into AnsiLineCollector.
        // When the process exits (EOF), waits for exit code and sends Completed.
        let mut reader = pair.master.try_clone_reader()?;
        let collector_clone = Arc::clone(&collector);
        let child_clone = Arc::clone(&child);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF — process exited
                    Ok(n) => {
                        if let Ok(mut guard) = collector_clone.lock() {
                            let (ref mut coll, ref mut parser) = *guard;
                            feed_through_vte(parser, coll, &buf[..n]);
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
            collector,
            input_tx,
            master: pair.master,
        })
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, bytes: &[u8]) {
        let _ = self.input_tx.send(bytes.to_vec());
    }

    /// Resize the PTY.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Get a snapshot of the styled output lines.
    pub fn snapshot(&self) -> Vec<StyledLine> {
        self.collector
            .lock()
            .map(|guard| guard.0.snapshot())
            .unwrap_or_default()
    }

    /// Get total line count.
    pub fn line_count(&self) -> usize {
        self.collector
            .lock()
            .map(|guard| guard.0.line_count())
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

    fn make_collector(max: usize) -> (AnsiLineCollector, vte::Parser) {
        (AnsiLineCollector::new(max), vte::Parser::new())
    }

    fn feed(collector: &mut AnsiLineCollector, parser: &mut vte::Parser, input: &[u8]) {
        feed_through_vte(parser, collector, input);
    }

    #[test]
    fn test_plain_text_line() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"hello\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let line_text: String = snap[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(line_text, "hello");
    }

    #[test]
    fn test_ansi_red_text() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"\x1b[31mERROR\x1b[0m\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let error_span = snap[0]
            .spans
            .iter()
            .find(|s| s.text.contains("ERROR"))
            .expect("Should have ERROR span");
        assert_eq!(error_span.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_256_color() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"\x1b[38;5;208mORANGE\x1b[0m\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let span = snap[0]
            .spans
            .iter()
            .find(|s| s.text.contains("ORANGE"))
            .expect("Should have ORANGE span");
        assert_eq!(span.style.fg, Some(Color::Indexed(208)));
    }

    #[test]
    fn test_rgb_color() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"\x1b[38;2;255;128;0mRGB\x1b[0m\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let span = snap[0]
            .spans
            .iter()
            .find(|s| s.text.contains("RGB"))
            .expect("Should have RGB span");
        assert_eq!(span.style.fg, Some(Color::Rgb(255, 128, 0)));
    }

    #[test]
    fn test_bold_and_color() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"\x1b[1;32mBOLD GREEN\x1b[0m\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let span = snap[0]
            .spans
            .iter()
            .find(|s| s.text.contains("BOLD GREEN"))
            .expect("Should have BOLD GREEN span");
        assert_eq!(span.style.fg, Some(Color::Green));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_multiple_lines() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"line1\nline2\nline3\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 3);
    }

    #[test]
    fn test_ring_buffer_cap() {
        let (mut c, mut p) = make_collector(5);
        for i in 0..10 {
            let line = format!("line{}\n", i);
            feed(&mut c, &mut p, line.as_bytes());
        }
        let snap = c.snapshot();
        assert_eq!(snap.len(), 5);
        let first_text: String = snap[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(first_text, "line5");
    }

    #[test]
    fn test_carriage_return() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"50%\r100%\n");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let line_text: String = snap[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(line_text, "100%");
    }

    #[test]
    fn test_partial_line() {
        let (mut c, mut p) = make_collector(100);
        feed(&mut c, &mut p, b"partial");
        let snap = c.snapshot();
        assert_eq!(snap.len(), 1);
        let text: String = snap[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "partial");
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
