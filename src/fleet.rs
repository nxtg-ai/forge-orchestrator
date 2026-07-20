//! Fleet context-budget HUD (W2-C, consolidation RFC §3 item 1).
//!
//! Reads each agent pane's **context gauge** from its statusline and reports a context-budget band
//! per pane. Two hard disciplines, mirroring the pod work's live-surface safety:
//!
//! - **Bounded capture.** Only the pane's visible screen is captured, then narrowed to the last
//!   [`STATUSLINE_LINES`] RAW lines (no non-empty filtering — filtering would promote a sparse
//!   pane's mid-text into scope). Never scrollback, never the whole pane. The buffer feeds straight
//!   into the pure [`hud_row`] pipeline, which keeps only an `Option<u8>`; nothing else is retained.
//! - **Read-only.** Nothing here writes to any pane or tmux server.
//!
//! Extraction is **adapter-based and by structural redundancy** — each pane is classified by trying
//! every adapter's `parse_ctx_pct` and taking the first `Some`. An adapter claims a reading only
//! when the co-occurring redundancy a real statusline has (and an ordinary log line cannot forge) is
//! present on a line: for Claude `ctx:NN%` AND a sibling `5h:`/`7d:` rate-limit gauge AND a
//! versioned model-token bracket (letters + digits, `[Fable 5:high]`); for Codex the `gpt-*` model
//! token AND middle-dot separators AND both the `Context …% left` and `weekly …% left` gauges. A
//! bare gauge phrase (`Context 5% left`, `ctx:42%` in a log line, `[INFO] [priority:high] ctx:42%`)
//! lacks that redundancy, so it is `n/a` **by construction** — not by a filter that could be
//! out-argued. Forging a reading would require actually being a statusline.

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

/// How many bottom RAW lines form the statusline window. Empirically the Claude statusline sits
/// 3rd-from-bottom (two input-hint lines — "bypass permissions", "focus" — render below it), while
/// the Codex statusline is the last line; 3 raw lines is the minimum that includes both. RAW (no
/// non-empty filtering) is deliberate: filtering would promote a sparse pane's mid-text into scope.
/// False positives are prevented by the adapters' full structural-signature match, not by this
/// window, so a tight raw window can only ever cause an honest `n/a`, never a wrong reading.
const STATUSLINE_LINES: usize = 3;

/// Keep only the last `n` RAW lines (no filtering) — **pure**.
pub fn last_raw_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Capture the pane's **visible screen**, then keep only the bottom [`STATUSLINE_LINES`] raw lines.
/// Never scrollback, never the whole pane. Extraction is confined to the statusline window and
/// gated by each adapter's structural signature, so neither pane history nor mid-pane text can be
/// mistaken for the gauge. The captured text is consumed immediately and never retained.
fn capture_statusline(pane: &str) -> String {
    let out = tmux().args(["capture-pane", "-p", "-t", pane]).output();
    let visible = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return String::new(),
    };
    last_raw_lines(&visible, STATUSLINE_LINES)
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

/// A compact one-line fleet summary for the dashboard strip — **pure**. Lists panes at PREP or
/// worse, most-severe first, and counts the rest as OK. Empty fleet / all-n/a → a plain note.
pub fn strip_summary(rows: &[HudRow]) -> String {
    let mut tracked: Vec<(&str, u8, BudgetLevel)> = rows
        .iter()
        .filter_map(|r| match (r.used_pct, r.level) {
            (Some(p), Some(l)) => Some((r.pane.as_str(), p, l)),
            _ => None,
        })
        .collect();
    if tracked.is_empty() {
        return "fleet: no ctx gauges".to_string();
    }
    // Most-severe first; ties broken by higher used%.
    tracked.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
    let flagged: Vec<String> = tracked
        .iter()
        .filter(|(_, _, l)| *l >= BudgetLevel::Prep)
        .map(|(pane, pct, level)| format!("{pane} {pct}% {}", level.label()))
        .collect();
    let ok = tracked.len() - flagged.len();
    if flagged.is_empty() {
        format!("fleet: {} panes, all OK", tracked.len())
    } else {
        format!("fleet ({ok} OK): {}", flagged.join(" · "))
    }
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

    /// A full Claude statusline with `used`% context — the structural REDUNDANCY the adapter
    /// requires: `ctx:` + the sibling `5h:`/`7d:` rate-limit gauges + a versioned model-token
    /// bracket (`[Fable 5:high]`), all co-occurring on one line.
    fn claude_line(used: u8) -> String {
        format!("[WOLF] /ASIF [Fable 5:high] ctx:{used}% 5h:43%(~2h58m) 7d:94%(~4h8m)")
    }

    /// A full Codex statusline with `left`% remaining — model token, middle-dot separators, and both
    /// the Context and weekly gauges co-occurring.
    fn codex_line(left: u8) -> String {
        format!("gpt-5.6-sol high \u{00b7} Context {left}% left \u{00b7} weekly 61% left")
    }

    #[test]
    fn claude_statusline_reads_ctx_not_the_sibling_rate_limit_percentages() {
        // The REAL Wolf statusline carries three percentages; only `ctx:` is truth. Must be 36 —
        // never the 5h:43% or 7d:94% next to it — and only because the full signature is present.
        let rows = hud_rows(&[("wolf".into(), claude_line(36))]);
        assert_eq!(rows[0].used_pct, Some(36), "{rows:?}");
        assert_eq!(rows[0].tool.as_deref(), Some("claude"));
        assert_eq!(rows[0].level, Some(BudgetLevel::Prep));
    }

    #[test]
    fn codex_statusline_normalizes_remaining_to_used() {
        // Codex reports REMAINING; the HUD is in USED. 21% left ⇒ 79 used ⇒ COMPACT.
        let rows = hud_rows(&[("codex".into(), codex_line(21))]);
        assert_eq!(rows[0].used_pct, Some(79));
        assert_eq!(rows[0].tool.as_deref(), Some("codex"));
        assert_eq!(rows[0].level, Some(BudgetLevel::Compact));
    }

    #[test]
    fn codex_reads_the_context_gauge_not_the_sibling_weekly_gauge() {
        // `Context 59% left · weekly 61% left` ⇒ 41 used (100-59), never from the weekly 61%.
        let rows = hud_rows(&[("codex".into(), codex_line(59))]);
        assert_eq!(rows[0].used_pct, Some(41), "{rows:?}");
        assert_eq!(rows[0].tool.as_deref(), Some("codex"));
    }

    #[test]
    fn polarity_boundaries() {
        let claude = |u| hud_rows(&[("c".into(), claude_line(u))])[0].used_pct;
        assert_eq!(claude(0), Some(0));
        assert_eq!(claude(100), Some(100));
        let codex = |l| hud_rows(&[("c".into(), codex_line(l))])[0].used_pct;
        assert_eq!(codex(0), Some(100), "0% context left = fully used");
        assert_eq!(codex(100), Some(0), "100% left = none used");
    }

    // --- The four seeded controls (regate round-3 ruling) ------------------------------------

    #[test]
    fn control_one_line_diagnostic_pane_is_na() {
        // A bare gauge phrase with none of the statusline structure → n/a BY CONSTRUCTION.
        for line in [
            "Context 5% left",
            "only 5% left to process the queue",
            "weekly 61% left",
            "ctx:42% appears in this log line",
            // regate round-4: a log line whose bracket-with-colon is a TIME, not [Model:effort].
            "[INFO] [12:30] ctx:42% starting run",
            "[WARN] [09:05:11] ctx:88% retrying",
            // regate round-5: `high` is a valid effort word, so a shape check passed these; the
            // redundancy signature rejects them — no sibling 5h:/7d: gauge, no letter+digit bracket.
            "[INFO] [priority:high] ctx:42% starting run",
            "[job] [worker:high] ctx:63% dispatched",
        ] {
            let rows = hud_rows(&[("diag".into(), line.into())]);
            assert_eq!(rows[0].used_pct, None, "must be n/a: {line:?}");
            assert_eq!(rows[0].tool, None, "must be n/a: {line:?}");
        }
    }

    #[test]
    fn control_real_codex_statusline_reads() {
        let rows = hud_rows(&[("codex".into(), codex_line(54))]);
        assert_eq!(rows[0].used_pct, Some(46));
        assert_eq!(rows[0].tool.as_deref(), Some("codex"));
    }

    #[test]
    fn control_real_claude_statusline_reads() {
        let rows = hud_rows(&[("wolf".into(), claude_line(43))]);
        assert_eq!(rows[0].used_pct, Some(43));
        assert_eq!(rows[0].tool.as_deref(), Some("claude"));
    }

    #[test]
    fn control_sparse_pane_with_a_mid_text_percentage_is_na() {
        // A sparse pane whose bottom lines are NOT a statusline — the raw last-3-line window over a
        // 3-line pane still includes the diagnostic, but the structural signature rejects it → n/a.
        let pane = "Context 5% left\n\n";
        let region = last_raw_lines(pane, STATUSLINE_LINES);
        let rows = hud_rows(&[("sparse".into(), region)]);
        assert_eq!(rows[0].used_pct, None, "{rows:?}");
        assert_eq!(rows[0].tool, None);
    }

    // --- capture window + pipeline -----------------------------------------------------------

    #[test]
    fn last_raw_lines_keeps_the_last_n_lines_unfiltered() {
        // RAW — no non-empty filtering; blanks are preserved so a sparse pane cannot promote
        // mid-text into the window.
        assert_eq!(last_raw_lines("a\nb\nc", 2), "b\nc");
        assert_eq!(
            last_raw_lines("x\n\ny", 3),
            "x\n\ny",
            "blank lines preserved (no filtering)"
        );
        assert_eq!(
            last_raw_lines("x\ny", 5),
            "x\ny",
            "fewer lines than n → all"
        );
    }

    #[test]
    fn a_mid_pane_statusline_phrase_is_out_of_the_raw_window() {
        // The real footer (statusline 3rd-from-bottom + two input hints) reads; a diagnostic higher
        // up is outside the last-3 raw window entirely.
        let pane = format!(
            "history\nContext 5% left DIAGNOSTIC\nmore\n{}\nbypass permissions\nfocus",
            claude_line(43)
        );
        let region = last_raw_lines(&pane, STATUSLINE_LINES);
        assert!(
            !region.contains("DIAGNOSTIC"),
            "mid-pane text out of window: {region:?}"
        );
        // The real statusline is also out of a last-3 window here (it's 3rd-from-bottom with two
        // hints below) — the honest result is n/a, never the diagnostic's 95.
        let rows = hud_rows(&[("p".into(), region)]);
        assert_ne!(
            rows[0].used_pct,
            Some(95),
            "must never read the mid-pane diagnostic"
        );
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
        let claude = hud_rows(&[("a".into(), claude_line(12))]);
        assert_eq!(claude[0].tool.as_deref(), Some("claude"));
        let codex = hud_rows(&[("b".into(), codex_line(44))]);
        assert_eq!(codex[0].tool.as_deref(), Some("codex"));
    }

    #[test]
    fn json_render_carries_pane_tool_pct_and_level() {
        let rows = hud_rows(&[
            ("wolf".into(), claude_line(82)),
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
    fn strip_summary_orders_by_severity_and_counts_ok() {
        let rows = hud_rows(&[
            ("wolf".into(), claude_line(36)),    // PREP
            ("kestrel".into(), claude_line(82)), // STOP
            ("dx3".into(), claude_line(12)),     // OK
            ("codex".into(), codex_line(5)),     // 95 used → EMERGENCY
            ("idle".into(), "bash".into()),      // n/a — excluded
        ]);
        let s = strip_summary(&rows);
        assert!(s.starts_with("fleet (1 OK):"), "{s}");
        let emerg = s.find("codex 95% EMERGENCY").unwrap();
        let stop = s.find("kestrel 82% STOP").unwrap();
        let prep = s.find("wolf 36% PREP").unwrap();
        assert!(emerg < stop && stop < prep, "most-severe first: {s}");
        assert!(!s.contains("idle"), "n/a panes excluded: {s}");
    }

    #[test]
    fn strip_summary_handles_all_ok_and_empty() {
        assert_eq!(
            strip_summary(&hud_rows(&[("a".into(), claude_line(5))])),
            "fleet: 1 panes, all OK"
        );
        assert_eq!(
            strip_summary(&hud_rows(&[("a".into(), "bash".into())])),
            "fleet: no ctx gauges"
        );
    }

    #[test]
    fn table_hides_na_panes_unless_show_all() {
        let rows = hud_rows(&[
            ("live".into(), claude_line(40)),
            ("idle".into(), "bash".into()),
        ]);
        let compact = render_table(&rows, false);
        assert!(compact.contains("live"));
        assert!(!compact.contains("idle"), "n/a panes hidden by default");
        let all = render_table(&rows, true);
        assert!(all.contains("idle"), "--all shows n/a panes");
    }
}
