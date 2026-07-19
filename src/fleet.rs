//! Fleet context-budget HUD (W2-C, consolidation RFC §3 item 1).
//!
//! Reads the **`ctx` gauge only** from each agent pane's statusline and reports a context-budget
//! band per pane. Two hard disciplines, mirroring the pod work's live-surface safety:
//!
//! - **Gauge-only.** The capture buffer is fed straight into the pure [`hud_row`] pipeline, which
//!   keeps only an `Option<u8>`; no other pane text is retained, logged, or surfaced.
//! - **Read-only.** Nothing here writes to any pane or tmux server.
//!
//! Extraction is **adapter-based** — each pane is classified by trying every adapter's
//! `parse_ctx_pct` and taking the first `Some`. The gauges are mutually unambiguous (`ctx:` prefix
//! vs `% left` suffix), so no brittle `pane_current_command → tool` mapping is needed; a pane whose
//! statusline matches no adapter is reported `n/a`, never guessed.

use crate::adapters::ToolAdapter;
use crate::core::budget::BudgetLevel;

/// One pane's context-budget line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudRow {
    pub pane: String,
    /// The adapter that recognized the gauge, or `None` when no gauge was found.
    pub tool: Option<String>,
    /// Context USED percent, or `None` (→ "n/a").
    pub used_pct: Option<u8>,
    pub level: Option<BudgetLevel>,
}

/// Assemble one row from a pane id and its captured statusline — **pure**. Tries each adapter and
/// takes the first that recognizes a gauge; keeps only the normalized percent (gauge-only).
pub fn hud_row(pane: &str, statusline: &str, adapters: &[&dyn ToolAdapter]) -> HudRow {
    for adapter in adapters {
        if let Some(used) = adapter.parse_ctx_pct(statusline) {
            return HudRow {
                pane: pane.to_string(),
                tool: Some(adapter.name().to_string()),
                used_pct: Some(used),
                level: Some(BudgetLevel::classify(used)),
            };
        }
    }
    HudRow {
        pane: pane.to_string(),
        tool: None,
        used_pct: None,
        level: None,
    }
}

/// The adapters consulted for gauge extraction, in order. Claude and Codex recognize gauges; Gemini
/// is included so an unrecognized pane is explicitly `n/a` rather than an error.
pub fn default_adapters() -> Vec<Box<dyn ToolAdapter>> {
    vec![
        Box::new(crate::adapters::claude::ClaudeAdapter),
        Box::new(crate::adapters::codex::CodexAdapter),
        Box::new(crate::adapters::gemini::GeminiAdapter),
    ]
}

/// Build HUD rows from captured `(pane, statusline)` specimens — **pure**, the whole testable core.
pub fn hud_rows(specimens: &[(String, String)]) -> Vec<HudRow> {
    let adapters = default_adapters();
    let refs: Vec<&dyn ToolAdapter> = adapters.iter().map(|a| a.as_ref()).collect();
    specimens
        .iter()
        .map(|(pane, statusline)| hud_row(pane, statusline, &refs))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// IO shell — tmux capture (read-only, gauge-only)
// ---------------------------------------------------------------------------------------------

/// Points tmux at a private server. Test isolation only — unset in production (default server).
pub const TMUX_SOCKET_ENV: &str = "FORGE_FLEET_TMUX_SOCKET";

/// How many trailing lines of a pane to capture. The Claude/Codex statusline renders at the bottom;
/// a small window catches it while minimizing how much is read. Empirically the gauge sat within
/// the last few visible rows across live specimens.
const STATUSLINE_WINDOW: u32 = 6;

fn tmux() -> std::process::Command {
    let mut cmd = std::process::Command::new("tmux");
    if let Some(socket) = std::env::var_os(TMUX_SOCKET_ENV).filter(|v| !v.is_empty()) {
        cmd.arg("-L").arg(socket);
    }
    cmd
}

/// List agent pane ids (`session:window.pane`). Read-only metadata.
fn list_panes() -> Vec<String> {
    let out = tmux()
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// Capture ONLY the gauge from a pane's statusline window, discarding everything else immediately.
///
/// Returns the row's normalized percent via the adapters — the captured text never leaves this
/// function. `None` when no gauge is present (→ n/a).
fn capture_statusline(pane: &str) -> String {
    let out = tmux()
        .args([
            "capture-pane",
            "-p",
            "-t",
            pane,
            "-S",
            &format!("-{STATUSLINE_WINDOW}"),
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Read the live fleet: every pane's ctx band. Read-only, gauge-only.
pub fn read_fleet() -> Vec<HudRow> {
    let specimens: Vec<(String, String)> = list_panes()
        .into_iter()
        .map(|pane| {
            let statusline = capture_statusline(&pane);
            (pane, statusline)
        })
        .collect();
    hud_rows(&specimens)
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

/// Human-readable table. Only panes with a recognized gauge OR (when `show_all`) every pane.
pub fn render_table(rows: &[HudRow], show_all: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "{:<28} {:<7} {:>5}  LEVEL", "PANE", "TOOL", "CTX%");
    for row in rows {
        if row.used_pct.is_none() && !show_all {
            continue;
        }
        let ctx = row
            .used_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "n/a".to_string());
        let level = row.level.map(|l| l.label()).unwrap_or("n/a");
        let tool = row.tool.as_deref().unwrap_or("-");
        let _ = writeln!(out, "{:<28} {:<7} {:>5}  {}", row.pane, tool, ctx, level);
    }
    out
}

/// Machine-readable JSON for scripts (the `--json` surface).
pub fn render_json(rows: &[HudRow]) -> String {
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "pane": r.pane,
                "tool": r.tool,
                "ctx_used_pct": r.used_pct,
                "level": r.level.map(|l| l.label()),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Array(arr)).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_statusline_reads_ctx_not_the_sibling_rate_limit_percentages() {
        // The single highest-value test: the REAL Wolf statusline carries three percentages and
        // only `ctx:` is the truth. Must be 36 — never the 5h:43% or 7d:94% next to it.
        let line = "[WOLF] /ASIF [Fable 5:high] ctx:36% 5h:43%(~2h58m) 7d:94%(~4h8m)      focus";
        let rows = hud_rows(&[("wolf".into(), line.into())]);
        assert_eq!(
            rows[0].used_pct,
            Some(36),
            "must anchor on ctx:, got {rows:?}"
        );
        assert_eq!(rows[0].tool.as_deref(), Some("claude"));
        assert_eq!(rows[0].level, Some(BudgetLevel::Prep));
    }

    #[test]
    fn codex_statusline_normalizes_remaining_to_used() {
        // Codex reports REMAINING; the HUD is in USED. 21% left ⇒ 79 used ⇒ COMPACT.
        let rows = hud_rows(&[("codex".into(), "gpt-5.6  Context 21% left".into())]);
        assert_eq!(rows[0].used_pct, Some(79));
        assert_eq!(rows[0].tool.as_deref(), Some("codex"));
        assert_eq!(rows[0].level, Some(BudgetLevel::Compact));
    }

    #[test]
    fn codex_polarity_boundaries() {
        let case = |s: &str| hud_rows(&[("c".into(), s.into())])[0].used_pct;
        assert_eq!(
            case("Context 0% left"),
            Some(100),
            "empty context = fully used"
        );
        assert_eq!(
            case("Context 100% left"),
            Some(0),
            "full context = none used"
        );
    }

    #[test]
    fn claude_boundaries() {
        let case = |s: &str| hud_rows(&[("c".into(), s.into())])[0].used_pct;
        assert_eq!(case("ctx:0%"), Some(0));
        assert_eq!(case("ctx:100%"), Some(100));
        assert_eq!(case("CTX:53%"), Some(53), "case-insensitive");
    }

    #[test]
    fn a_pane_with_no_gauge_is_na_never_guessed_or_an_error() {
        let rows = hud_rows(&[("idle".into(), "  bash  /home/axw/projects".into())]);
        assert_eq!(rows[0].used_pct, None);
        assert_eq!(rows[0].tool, None);
        assert_eq!(rows[0].level, None);
    }

    #[test]
    fn adapters_do_not_cross_claim_each_others_gauges() {
        // Non-overlapping by construction: the claude line has no `% left`, the codex line no `ctx:`.
        let claude = hud_rows(&[("a".into(), "ctx:12%".into())]);
        assert_eq!(claude[0].tool.as_deref(), Some("claude"));
        let codex = hud_rows(&[("b".into(), "44% left".into())]);
        assert_eq!(codex[0].tool.as_deref(), Some("codex"));
    }

    #[test]
    fn json_render_carries_pane_tool_pct_and_level() {
        let rows = hud_rows(&[
            ("wolf".into(), "ctx:82%".into()),
            ("idle".into(), "bash".into()),
        ]);
        let json = render_json(&rows);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v[0]["ctx_used_pct"], 82);
        assert_eq!(v[0]["level"], "STOP");
        assert_eq!(v[1]["ctx_used_pct"], serde_json::Value::Null);
        assert_eq!(v[1]["level"], serde_json::Value::Null);
    }

    #[test]
    fn table_hides_na_panes_unless_show_all() {
        let rows = hud_rows(&[
            ("live".into(), "ctx:40%".into()),
            ("idle".into(), "bash".into()),
        ]);
        let compact = render_table(&rows, false);
        assert!(compact.contains("live"));
        assert!(!compact.contains("idle"), "n/a panes hidden by default");
        let all = render_table(&rows, true);
        assert!(all.contains("idle"), "--all shows n/a panes");
    }
}
