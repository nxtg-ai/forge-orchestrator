# Assignment: DX-037 + DX-038 — Quota Monitoring + Subscription Risk Warning

> **Scope:** CODE CHANGES. Implement both DX items in forge-orchestrator.
> **Priority:** CRITICAL — v1.3.0 "The Safe Operator"
> **Tests:** Must add tests for all new functionality. Target: 210+ total tests (currently 200).
> **Build:** Must compile clean (`cargo build --release`), clippy clean, `cargo fmt` applied.

## Context

RESEARCH-001 (see `docs/research/cli-subscription-gating-analysis.md`) confirmed:
- Anthropic ACTIVELY BLOCKS subscription-credential CLI orchestration (Jan 2026 crackdown, account bans)
- API-key mode is safe and blessed by ALL providers
- Users need visibility into quota consumption and risk warnings

## DX-037: Quota Monitoring

### What to build

Track per-provider task dispatch count and display in dashboard footer + `forge status`.

### Data Model

Add to `App` struct in `src/tui/app.rs`:

```rust
/// Per-provider task dispatch count for quota monitoring (DX-037).
/// Key: AgentType, Value: (dispatched_count, window_start)
pub provider_quota: HashMap<AgentType, (u32, Instant)>,
```

Initialize in `App::new()`:
```rust
provider_quota: HashMap::new(),
```

### Quota Tracking

In `spawn_task()`, after successful spawn (inside `Ok(mut child)` arm), increment the quota counter:

```rust
// DX-037: Track quota usage
let quota = self.provider_quota
    .entry(agent.clone())
    .or_insert((0, Instant::now()));
// Reset counter if 5-hour window has elapsed
if quota.1.elapsed() > std::time::Duration::from_secs(5 * 3600) {
    *quota = (0, Instant::now());
}
quota.0 += 1;
```

### Dashboard Footer

Modify `render_footer()` in `src/tui/ui.rs` to show quota info in a **second footer line**. Change the layout from `Constraint::Length(1)` to `Constraint::Length(2)` for the footer.

The quota footer line format:
```
Claude: 3/50 (5h) │ Codex: 8/60 (5h) │ Gemini: 45/1000 (RPD)
```

Where the denominator is:
- Claude subscription: 50 (conservative estimate for Max 5x)
- Claude API: show "API" instead of a number
- Codex subscription: 60 (max cloud tasks/5h for Team)
- Codex API: show "API"
- Gemini subscription: 1000 (RPD for Google Account)
- Gemini API: show "API"

Color coding:
- Green: < 50% of quota
- Yellow: 50-80% of quota
- Red: > 80% of quota
- Cyan: API mode (no quota concern)

Implementation in `render_footer()`:

```rust
fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    // Split footer into 2 lines: keys + quota
    let footer_chunks = Layout::vertical([
        Constraint::Length(1), // Key legend
        Constraint::Length(1), // Quota line
    ]).split(area);

    // Line 1: Key legend (existing code)
    // ... existing text logic ...
    f.render_widget(key_paragraph, footer_chunks[0]);

    // Line 2: Quota monitoring
    let quota_spans = build_quota_spans(app);
    let quota_line = Paragraph::new(Line::from(quota_spans));
    f.render_widget(quota_line, footer_chunks[1]);
}
```

Add helper function to `ui.rs`:

```rust
fn build_quota_spans(app: &App) -> Vec<Span<'static>> {
    let state_mgr = StateManager::new(&app.forge_dir);

    let mut spans = Vec::new();
    for (idx, agent_type) in [AgentType::Claude, AgentType::Codex, AgentType::Gemini].iter().enumerate() {
        let agent_name = agent_type.to_string().to_lowercase();
        let auth_mode = state_mgr
            .get_agent_auth(&agent_name)
            .unwrap_or_else(|_| "subscription".to_string());

        let (count, _) = app.provider_quota
            .get(agent_type)
            .copied()
            .unwrap_or((0, Instant::now()));

        if auth_mode == "api" {
            spans.push(Span::styled(
                format!("{}: {} (API)", agent_type, count),
                Style::default().fg(Color::Cyan),
            ));
        } else {
            let max = match agent_type {
                AgentType::Claude => 50,
                AgentType::Codex => 60,
                AgentType::Gemini => 1000,
                _ => 100,
            };
            let ratio = count as f32 / max as f32;
            let color = if ratio > 0.8 { Color::Red }
                       else if ratio > 0.5 { Color::Yellow }
                       else { Color::Green };
            spans.push(Span::styled(
                format!("{}: {}/{} (5h)", agent_type, count, max),
                Style::default().fg(color),
            ));
        }

        if idx < 2 {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        }
    }
    spans
}
```

### `forge status` Integration

In `src/cli/status.rs`, add a "Quota" section after the "Agent Configuration" section:

```rust
// Quota info (DX-037)
println!("  {}", "Provider Quota (this session):".bold());
println!("    ⚠ Quota tracking is only active during dashboard sessions");
println!("    Use `forge dashboard` or `forge start` for live quota monitoring");
```

(Full quota tracking only works in dashboard mode since it requires a running process. `forge status` just explains this.)

### Tests

Add to `src/tui/app.rs` tests:

1. `test_quota_counter_increments` — verify quota counter increases after spawn simulation
2. `test_quota_window_resets_after_5h` — verify counter resets when window expires
3. `test_quota_separate_per_agent` — verify each agent has independent counter

Add to `src/tui/ui.rs` or create inline test for `build_quota_spans`:

4. `test_quota_spans_api_mode` — verify API mode shows cyan "(API)" label
5. `test_quota_spans_subscription_colors` — verify green/yellow/red color coding

---

## DX-038: Subscription Risk Warning

### What to build

1. Warning when setting `claude.auth subscription` via config
2. Warning banner in dashboard header when ANY provider uses subscription mode
3. Requirement for `--i-accept-subscription-risk` flag in `forge dashboard` and `forge start`

### Config Warning (`src/cli/config.rs`)

When `claude.auth subscription` is set, show the research-backed warning:

In the `"claude.auth" | "codex.auth" | "gemini.auth"` match arm, when value is "subscription", add after the existing println:

```rust
"subscription" => {
    println!("  → Will use CLI subscription (API keys stripped from subprocess)");

    // DX-038: Risk warning based on RESEARCH-001
    if agent == "claude" {
        println!();
        println!("  ⚠⚠⚠  WARNING: SUBSCRIPTION RISK  ⚠⚠⚠");
        println!("  Anthropic ACTIVELY BLOCKS subscription-based CLI orchestration.");
        println!("  In January 2026, accounts were banned for using third-party");
        println!("  tools with subscription OAuth tokens (ToS Section D.4).");
        println!("  → STRONGLY RECOMMENDED: Use `forge config claude.auth api` instead.");
        println!("  → See: docs/research/cli-subscription-gating-analysis.md");
        println!();
        println!("  To use subscription mode in dashboard/start, you must pass:");
        println!("  `forge dashboard --i-accept-subscription-risk`");
    } else if agent == "codex" {
        println!();
        println!("  ⚠ CAUTION: Codex subscription has 10-60 cloud task/5h limits.");
        println!("  Consider API key mode for heavier workloads.");
    }
}
```

### Dashboard Header Warning (`src/tui/ui.rs`)

In `render_task_board()`, if any provider uses subscription auth, show a warning line in the title:

```rust
// Check for subscription risk (DX-038)
let state_mgr_warn = StateManager::new(&app.forge_dir);
let has_sub_risk = ["claude", "codex", "gemini"].iter().any(|agent| {
    state_mgr_warn
        .get_agent_auth(agent)
        .unwrap_or_else(|_| "subscription".to_string())
        == "subscription"
});
```

If `has_sub_risk` is true, change the border color to Yellow and append "⚠ SUB" to the title string.

### CLI Flag Gate (`src/cli/dashboard.rs` + `src/cli/start.rs`)

Add `--i-accept-subscription-risk` flag to the dashboard and start commands.

In `src/cli/dashboard.rs`, before launching the TUI, check if any provider uses subscription mode:

```rust
// DX-038: Check for subscription risk
let state_mgr = StateManager::new(&forge_dir);
let state = state_mgr.load()?;
let has_claude_sub = state.agent_auth.get("claude")
    .map(|v| v == "subscription")
    .unwrap_or(true); // default is subscription

if has_claude_sub && !accept_subscription_risk {
    println!();
    println!("  ⚠⚠⚠  SUBSCRIPTION RISK DETECTED  ⚠⚠⚠");
    println!();
    println!("  Claude is configured to use subscription auth.");
    println!("  Anthropic ACTIVELY BLOCKS third-party CLI orchestration");
    println!("  and has BANNED accounts for this pattern.");
    println!();
    println!("  Options:");
    println!("    1. Switch to API mode (RECOMMENDED):");
    println!("       forge config claude.auth api");
    println!();
    println!("    2. Accept the risk:");
    println!("       forge dashboard --i-accept-subscription-risk");
    println!();
    println!("  See: docs/research/cli-subscription-gating-analysis.md");
    return Ok(());
}
```

The `accept_subscription_risk` parameter must be added to the CLI args. In `src/main.rs`, find the Dashboard subcommand and add:

```rust
/// Accept subscription risk for providers that may ban automated usage
#[arg(long = "i-accept-subscription-risk", default_value_t = false)]
accept_subscription_risk: bool,
```

Same for the `Start` subcommand.

### Tests

Add tests:

6. `test_subscription_risk_detected_claude` — verify Claude subscription auth is flagged as risky
7. `test_no_risk_with_api_mode` — verify API mode does not trigger warning
8. `test_subscription_risk_codex_no_block` — verify Codex subscription shows caution but doesn't block
9. `test_config_warning_claude_subscription` — verify config command shows warning text

---

## File Summary

| File | Changes |
|------|---------|
| `src/tui/app.rs` | Add `provider_quota` field, increment in `spawn_task`, add quota tests |
| `src/tui/ui.rs` | Add `build_quota_spans`, modify `render_footer` to 2 lines, add sub warning to header, add `StateManager` import |
| `src/cli/config.rs` | Add DX-038 subscription risk warnings in config set |
| `src/cli/dashboard.rs` | Add `--i-accept-subscription-risk` flag check |
| `src/cli/start.rs` | Add `--i-accept-subscription-risk` flag check |
| `src/main.rs` | Add `accept_subscription_risk` to Dashboard and Start CLI args |
| `src/cli/status.rs` | Add quota info note |

## Build & Test

```bash
cargo fmt
cargo clippy -- -W clippy::all
cargo test
cargo build --release
```

All 210+ tests must pass. Zero clippy warnings. Binary deploys to `~/.local/bin/forge-orca`.

## IMPORTANT NOTES

- Do NOT change any existing behavior — only ADD new features
- Do NOT modify adapter behavior (Claude/Codex/Gemini adapters stay the same)
- The footer layout change from Length(1) to Length(2) affects ALL footer rendering paths
- Import `StateManager` in `ui.rs` if not already imported
- The `Instant` type is already imported in `ui.rs` (check and add if needed)
- Use `use crate::core::task::AgentType;` in `ui.rs` if not already imported
- `cargo fmt` MUST be the last step before build

---

**CHECKPOINT: 210+ tests, clippy clean, cargo fmt applied, binary deployed.**
