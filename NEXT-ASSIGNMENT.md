# Assignment: DX-029 — Live Agent Streaming in TUI Dashboard

> **Scope:** 1 part. Run `cargo test && cargo clippy -- -D warnings` when done.
> **Version target:** Stay at v1.1.0 (patch, not bump)

## Problem

The TUI dashboard spawns Claude agents via `claude -p --output-format text`. The `text` format buffers ALL output and only dumps it when the process exits. This means the agent panes in the dashboard show nothing while agents work — they appear dead even though `ps aux` shows them running.

## Solution

Switch Claude adapter to `--output-format stream-json` which emits NDJSON (one JSON object per line) in real-time. Parse each line and extract human-readable activity text for the TUI panes.

### Stream-JSON Format

Each stdout line is a JSON object with this structure:

```typescript
interface StreamMessage {
  type: 'init' | 'message' | 'tool_use' | 'tool_result' | 'result';
  session_id?: string;
  // For type=message:
  role?: 'assistant' | 'user';
  content?: Array<{ type: 'text' | 'tool_use'; text?: string; name?: string; input?: any }>;
  // For type=result:
  status?: 'success' | 'error';
  duration_ms?: number;
}
```

### What to display in agent panes

Parse each NDJSON line and produce readable output:

| Event type | Display text |
|-----------|-------------|
| `init` | `[session started]` |
| `message` with `content[].type == "text"` | The text content (may be multi-line, split and push each line) |
| `message` with `content[].type == "tool_use"` | `[Tool] name: first_60_chars_of_input` e.g. `[Read] src/main.rs` or `[Bash] npm test` or `[Edit] src/foo.ts` |
| `tool_result` | Skip (too verbose) |
| `result` | `[done] status in Xms` |
| Parse failure | Push the raw line as-is (fallback) |

---

## Changes

### 1. Claude adapter (`src/adapters/claude.rs`)

Change line 90:

```rust
// BEFORE:
cmd.args(["-p", &prompt, "--output-format", "text"])

// AFTER:
cmd.args(["-p", &prompt, "--output-format", "stream-json"])
```

That's it for the adapter. The `execute_headless()` path (used by `forge run`) also needs updating — but `execute_headless` in `mod.rs` calls `.output()` which collects all stdout. For headless mode, keep `text` format. So instead, make `build_command` accept an optional `streaming: bool` parameter:

```rust
fn build_command(
    &self,
    task: &Task,
    project_root: &Path,
    auth_mode: &str,
    permissions: &str,
) -> Command {
    // ... existing code ...

    let mut cmd = Command::new("claude");
    cmd.args(["-p", &prompt, "--output-format", "stream-json"])
        .current_dir(project_root);

    // ... rest unchanged ...
}
```

Wait — actually the ToolAdapter trait has `build_command` returning a single Command. The dashboard uses `build_command` + manual spawn. The headless path uses `execute_headless` which calls `build_command` + `.output()`.

**The simplest fix:** Just change to `stream-json` in `build_command`. The headless path will still work — `.output()` collects all stdout lines, they'll just be JSONL instead of plain text. The `process_output` function in `mod.rs` combines stdout into a string regardless of format. The headless output won't be as pretty but it works.

If you want headless to remain clean text, add a separate method or a parameter. But for now, just change the format flag — the dashboard is the priority.

### 2. New streaming parser (`src/tui/app.rs`)

Replace the `stream_lines` function (or add a new `stream_claude_json` alongside it):

```rust
/// Parse Claude's stream-json NDJSON output into human-readable lines for TUI display.
async fn stream_claude_json(
    reader: impl tokio::io::AsyncRead + Unpin,
    task_id: String,
    agent: AgentType,
    tx: mpsc::UnboundedSender<AgentEvent>,
) {
    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let display_lines = parse_stream_json_line(&line);
        for display_line in display_lines {
            if tx
                .send(AgentEvent::Output {
                    task_id: task_id.clone(),
                    agent: agent.clone(),
                    line: display_line,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// Parse a single NDJSON line from Claude's stream-json output.
/// Returns zero or more human-readable display lines.
fn parse_stream_json_line(line: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        // Not valid JSON — show raw line as fallback
        return if line.trim().is_empty() {
            vec![]
        } else {
            vec![line.to_string()]
        };
    };

    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "init" => {
            vec!["[session started]".to_string()]
        }
        "message" => {
            let mut result = Vec::new();
            if let Some(content) = value.get("content").and_then(|c| c.as_array()) {
                for item in content {
                    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match item_type {
                        "text" => {
                            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                // Split multi-line text into separate display lines
                                for text_line in text.lines() {
                                    let trimmed = text_line.trim();
                                    if !trimmed.is_empty() {
                                        result.push(trimmed.to_string());
                                    }
                                }
                            }
                        }
                        "tool_use" => {
                            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let input_summary = summarize_tool_input(name, item.get("input"));
                            result.push(format!("[{}] {}", name, input_summary));
                        }
                        _ => {}
                    }
                }
            }
            result
        }
        "result" => {
            let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let duration = value.get("duration_ms").and_then(|v| v.as_u64());
            match duration {
                Some(ms) => vec![format!("[done] {} in {:.1}s", status, ms as f64 / 1000.0)],
                None => vec![format!("[done] {}", status)],
            }
        }
        // Skip tool_result (too verbose) and unknown types
        _ => vec![],
    }
}

/// Extract a short summary from tool input for display.
fn summarize_tool_input(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };

    match tool_name {
        "Read" => {
            input.get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Edit" => {
            input.get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Write" => {
            input.get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Bash" => {
            let cmd = input.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            truncate_str(cmd, 60)
        }
        "Glob" => {
            input.get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Grep" => {
            let pattern = input.get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            truncate_str(pattern, 40)
        }
        "Task" => {
            let desc = input.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("subagent");
            truncate_str(desc, 50)
        }
        _ => {
            // Generic: show first string value found
            if let Some(obj) = input.as_object() {
                for (_key, val) in obj.iter().take(1) {
                    if let Some(s) = val.as_str() {
                        return truncate_str(s, 50);
                    }
                }
            }
            String::new()
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
```

### 3. Use the JSON streamer in `spawn_task` (`src/tui/app.rs`)

In the `spawn_task` method, when spawning stdout reader for Claude agents, use `stream_claude_json` instead of `stream_lines`:

```rust
// Around line 1128-1132, change:
if let Some(stdout) = child.stdout.take() {
    let tx_out = tx.clone();
    let tid = task_id.clone();
    let ag = agent.clone();

    // Use JSON parser for Claude, plain lines for others
    match agent {
        AgentType::Claude | AgentType::Any => {
            tokio::spawn(stream_claude_json(stdout, tid, ag, tx_out));
        }
        _ => {
            tokio::spawn(stream_lines(stdout, tid, ag, tx_out));
        }
    }
}
```

Keep `stream_lines` for stderr (all agents) — stderr is always plain text.

### 4. Rate limit detection update

The existing rate limit detection in `handle_event` checks agent output lines for patterns like "rate limit", "429", etc. This still works because `parse_stream_json_line` extracts text content from messages — if Claude writes about rate limits, those strings will appear in the extracted text. No changes needed.

### 5. Tests

Add tests in `src/tui/app.rs` tests module:

```rust
#[test]
fn test_parse_stream_json_init() {
    let lines = parse_stream_json_line(r#"{"type":"init","session_id":"abc"}"#);
    assert_eq!(lines, vec!["[session started]"]);
}

#[test]
fn test_parse_stream_json_text_message() {
    let lines = parse_stream_json_line(
        r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"Hello world"}]}"#
    );
    assert_eq!(lines, vec!["Hello world"]);
}

#[test]
fn test_parse_stream_json_tool_use() {
    let lines = parse_stream_json_line(
        r#"{"type":"message","content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/main.rs"}}]}"#
    );
    assert_eq!(lines, vec!["[Read] src/main.rs"]);
}

#[test]
fn test_parse_stream_json_tool_use_bash() {
    let lines = parse_stream_json_line(
        r#"{"type":"message","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}"#
    );
    assert_eq!(lines, vec!["[Bash] npm test"]);
}

#[test]
fn test_parse_stream_json_result() {
    let lines = parse_stream_json_line(
        r#"{"type":"result","status":"success","duration_ms":45000}"#
    );
    assert_eq!(lines, vec!["[done] success in 45.0s"]);
}

#[test]
fn test_parse_stream_json_tool_result_skipped() {
    let lines = parse_stream_json_line(r#"{"type":"tool_result","output":"lots of stuff"}"#);
    assert!(lines.is_empty());
}

#[test]
fn test_parse_stream_json_invalid_json_fallback() {
    let lines = parse_stream_json_line("not json at all");
    assert_eq!(lines, vec!["not json at all"]);
}

#[test]
fn test_parse_stream_json_empty_line() {
    let lines = parse_stream_json_line("");
    assert!(lines.is_empty());
}

#[test]
fn test_summarize_tool_input_read() {
    let input: serde_json::Value = serde_json::json!({"file_path": "src/main.rs"});
    assert_eq!(summarize_tool_input("Read", Some(&input)), "src/main.rs");
}

#[test]
fn test_summarize_tool_input_bash_truncate() {
    let long_cmd = "a]".repeat(50);
    let input: serde_json::Value = serde_json::json!({"command": long_cmd});
    let result = summarize_tool_input("Bash", Some(&input));
    assert!(result.len() <= 63); // 60 + "..."
    assert!(result.ends_with("..."));
}

#[test]
fn test_truncate_str_short() {
    assert_eq!(truncate_str("hello", 10), "hello");
}

#[test]
fn test_truncate_str_long() {
    assert_eq!(truncate_str("hello world this is long", 10), "hello w...");
}
```

---

## Files Summary

| File | Action | What |
|------|--------|------|
| `src/adapters/claude.rs` | Modify line 90 | `"text"` → `"stream-json"` |
| `src/tui/app.rs` | Add functions | `stream_claude_json`, `parse_stream_json_line`, `summarize_tool_input`, `truncate_str` |
| `src/tui/app.rs` | Modify `spawn_task` | Use `stream_claude_json` for Claude stdout |
| `src/tui/app.rs` | Add tests | 12 new tests for JSON parsing |

## IMPORTANT NOTES

- Do NOT change the `stream_lines` function — it's still used for Codex, Gemini, and stderr
- Do NOT change `execute_headless` in `mod.rs` — headless mode can consume JSONL stdout fine
- The `serde_json` crate is already a dependency (used by task/finding serialization)
- Keep the existing rate limit detection logic — it reads from `agent_outputs` which will now contain parsed text, so it still works
- `parse_stream_json_line` and `summarize_tool_input` must be `pub(crate)` or at least non-private so tests can access them (or put them in the tests module scope)

---

## BONUS: Two DX Fixes (do these after the streaming work)

### DX-030: Project Name in Dashboard Header

Currently the header shows: `FORGE DASHBOARD — VERIFY (3/13)`

It should show: `FORGE DASHBOARD — voice-jib-jab — VERIFY (3/13)`

The project name is available in `App` — it's loaded from `.forge/state.json` (`project_name` field). Find where the header title is rendered in `src/tui/ui.rs` (look for `"FORGE DASHBOARD"`) and insert the project name between the product name and the phase.

If `App` doesn't currently store the project name, add a `pub project_name: String` field, load it from `StateManager` during `App::new()`, and use it in the header.

### DX-031: Freeze Timer on Completion

Currently the completion banner says: `All 34/34 tasks completed in 819m 46s`

The problem: that timer keeps ticking after completion because it's computed from `self.start_time` (an `Instant`) to `Instant::now()`. If you leave the dashboard open, the number grows forever.

Fix: Add a `pub completed_at: Option<Instant>` field to `App`. When `all_complete` becomes true (in the completion detection logic), snapshot `completed_at = Some(Instant::now())`. Then in the completion banner rendering, use `completed_at.unwrap_or_else(Instant::now)` instead of `Instant::now()` to compute elapsed time.

This freezes the timer at the moment of completion.

### Tests for bonus fixes

- Test that project name appears in header render output
- Test that `completed_at` is `None` initially and `Some` after all tasks complete
- Test that elapsed time uses `completed_at` when set

---

**CHECKPOINT: `cargo test && cargo clippy -- -D warnings` must pass.**
