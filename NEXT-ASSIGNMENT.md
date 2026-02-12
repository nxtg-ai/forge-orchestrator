# Next Assignment: DX-027 — User-Spawnable Shell Panes

> Priority: HIGH | Estimated: 1-1.5h | Version target: v0.3.7

## The Problem

User can't open a terminal while agents work. They're stuck watching. Want to run `ls`, `git status`, `npm test` etc. while waiting. Currently the 4th pane (index 3) is always "Summary" — it could be replaced with a user shell or a 5th pane added.

## The Fix

### Approach: Replace Summary pane with a user shell on demand

The Summary pane shows running/completed/failed counts. This info is already visible in the task board. When the user presses `+` or `s`, replace the Summary pane (index 3) with an interactive shell. Press `Ctrl+D` or type `exit` to close the shell and restore Summary.

### Part 1: Add shell process tracking to App (`src/tui/app.rs`)

Add fields to track the shell:

```rust
use tokio::process::Child;

pub struct App {
    // ... existing fields ...
    /// User shell process (replaces Summary pane when active)
    pub shell_active: bool,
    pub shell_output: VecDeque<String>,
    pub shell_input_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}
```

Initialize in `App::new()`:
```rust
shell_active: false,
shell_output: VecDeque::new(),
shell_input_tx: None,
```

### Part 2: Spawn shell command (`src/tui/app.rs`)

Add a method to spawn a shell:

```rust
pub fn spawn_shell(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
    if self.shell_active {
        return; // Already have a shell
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

    let mut cmd = TokioCommand::new(&shell);
    cmd.current_dir(&self.project_root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    match cmd.spawn() {
        Ok(mut child) => {
            // Create input channel for sending keystrokes to shell stdin
            let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            self.shell_input_tx = Some(input_tx);

            // Pipe stdin
            if let Some(mut stdin) = child.stdin.take() {
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    while let Some(text) = input_rx.recv().await {
                        if stdin.write_all(text.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = stdin.flush().await;
                    }
                });
            }

            // Pipe stdout to shell_output buffer via AgentEvent
            // Reuse the existing Output event with a special "shell" task_id
            if let Some(stdout) = child.stdout.take() {
                let tx_out = tx.clone();
                tokio::spawn(async move {
                    let reader = tokio::io::BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = tx_out.send(AgentEvent::Output {
                            task_id: "__shell__".to_string(),
                            agent: AgentType::Any, // Marker for shell
                            line,
                        });
                    }
                });
            }

            // Same for stderr
            if let Some(stderr) = child.stderr.take() {
                let tx_err = tx.clone();
                tokio::spawn(async move {
                    let reader = tokio::io::BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = tx_err.send(AgentEvent::Output {
                            task_id: "__shell__".to_string(),
                            agent: AgentType::Any,
                            line,
                        });
                    }
                });
            }

            // Watch for exit
            let tx_done = tx.clone();
            tokio::spawn(async move {
                let _ = child.wait().await;
                let _ = tx_done.send(AgentEvent::Completed {
                    task_id: "__shell__".to_string(),
                    agent: AgentType::Any,
                    success: true,
                    exit_code: 0,
                });
            });

            self.shell_active = true;
            self.shell_output.clear();
            self.shell_output.push_back(format!("$ {shell}"));
            self.push_event("User shell opened (Ctrl+D to close)");
        }
        Err(e) => {
            self.push_event(&format!("Failed to spawn shell: {}", e));
        }
    }
}
```

### Part 3: Handle shell output in `handle_agent_event()`

In the `AgentEvent::Output` handler, check for the special `__shell__` task_id:

```rust
AgentEvent::Output { task_id, agent, line } => {
    if task_id == "__shell__" {
        // Route to shell output buffer
        self.shell_output.push_back(line);
        while self.shell_output.len() > OUTPUT_BUFFER_CAP {
            self.shell_output.pop_front();
        }
    } else {
        // Existing agent output handling...
    }
}
```

In `AgentEvent::Completed`, check for shell exit:
```rust
if task_id == "__shell__" {
    self.shell_active = false;
    self.shell_input_tx = None;
    self.push_event("User shell closed");
    return Ok(());
}
```

### Part 4: Route keystrokes to shell when focused (`handle_key`)

When pane 3 is focused AND shell is active, route character keys to the shell stdin:

```rust
// In handle_key, when processing key events:
if let FocusArea::Pane(3) = self.focus {
    if self.shell_active {
        if let Some(tx) = &self.shell_input_tx {
            match key.code {
                KeyCode::Char(c) => { let _ = tx.send(c.to_string()); return; }
                KeyCode::Enter => { let _ = tx.send("\n".to_string()); return; }
                KeyCode::Backspace => { let _ = tx.send("\x7f".to_string()); return; }
                // Ctrl+D = close shell
                KeyCode::Char('d') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    let _ = tx.send("exit\n".to_string());
                    return;
                }
                // Ctrl+C = send interrupt
                KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    let _ = tx.send("\x03".to_string());
                    return;
                }
                // Tab in shell = tab character, not focus switch
                KeyCode::Tab => { let _ = tx.send("\t".to_string()); return; }
                // Escape = leave shell focus (back to dashboard keys)
                KeyCode::Esc => { /* fall through to normal Esc handling */ }
                _ => {}
            }
        }
    }
}
```

**Important:** Esc should NOT go to the shell — it should exit focus (return to dashboard key mode). This lets the user escape back to navigating panes. Ctrl+D closes the shell entirely.

### Part 5: Key binding for spawning shell

In `handle_key`, add the `s` or `+` binding when NOT in expanded mode and NOT focused on shell:

```rust
KeyCode::Char('s') | KeyCode::Char('+') => {
    if !self.shell_active {
        self.spawn_shell(agent_tx);
        self.focus = FocusArea::Pane(3); // Focus the shell pane
    } else {
        self.focus = FocusArea::Pane(3); // Just focus existing shell
    }
}
```

### Part 6: Update pane rendering (`src/tui/ui.rs`)

In `render_single_pane`, when `idx == 3`:
- If `app.shell_active` → render shell pane (title "Shell", green border when focused, show `shell_output` buffer)
- Else → render Summary pane as before

```rust
fn render_single_pane(f: &mut Frame, app: &App, idx: usize, area: Rect, expanded: bool) {
    // Check if this is the shell pane
    if idx == 3 && app.shell_active {
        render_shell_pane(f, app, area, expanded);
        return;
    }
    // ... existing pane rendering ...
}

fn render_shell_pane(f: &mut Frame, app: &App, area: Rect, expanded: bool) {
    let focused = app.focus == FocusArea::Pane(3);
    let border_style = if focused || expanded {
        Style::default().fg(Color::Green) // Green for shell
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if focused {
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

    // Render shell output
    let visible_lines = inner.height as usize;
    let total = app.shell_output.len();
    let skip = total.saturating_sub(visible_lines);
    let lines: Vec<Line> = app.shell_output.iter()
        .skip(skip)
        .map(|s| Line::from(s.as_str()))
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
```

### Part 7: Update key legend (`render_footer`)

When shell is not active, add `s:Shell` to the footer hints. When shell is active and focused, show `Esc:Unfocus | Ctrl+D:Close`.

### Part 8: Update `pane_label` for shell

```rust
pub fn pane_label(index: usize, shell_active: bool) -> &'static str {
    match index {
        0 => "Claude",
        1 => "Codex",
        2 => "Gemini",
        3 if shell_active => "Shell",
        3 => "Summary",
        _ => "?",
    }
}
```

Or simpler: just check `app.shell_active` in the render function directly.

## Files to modify

1. `src/tui/app.rs` — Shell state fields, `spawn_shell()`, shell key routing, `s`/`+` binding, shell output/exit handling
2. `src/tui/ui.rs` — `render_shell_pane()`, update footer hints, pane 3 conditional rendering

## Tests

- `spawn_shell` sets `shell_active = true` and clears output buffer
- Shell output routes to `shell_output` (not `agent_outputs`)
- `__shell__` completion resets `shell_active` to false
- `s` key spawns shell and focuses pane 3
- Esc in shell pane moves focus back to TaskBoard (doesn't send to shell)
- Ctrl+D in shell sends "exit\n" to shell stdin
- Shell not spawnable when already active (idempotent)
- Pane 3 renders Summary when shell inactive, Shell when active

## Verification

- [ ] `cargo test` — all pass
- [ ] `cargo clippy -- -D warnings` — 0 warnings
- [ ] Press `s` → shell appears in pane 3 with $ prompt
- [ ] Type commands → output visible in pane
- [ ] Esc → back to dashboard navigation
- [ ] Ctrl+D → shell closes, Summary pane returns
