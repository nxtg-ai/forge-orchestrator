//! Context-budget thresholds — the deterministic classification behind the W2-C fleet HUD.
//!
//! The percentages are **context USED** (0..=100), normalized by the per-tool adapters so Claude's
//! `ctx:NN%` and Codex's `NN% left` land on one scale. The thresholds mirror the portfolio
//! token-budget canon (`~/.claude/rules/token-budget-canon.md`): on a 1M-context pane, ctx% maps to
//! the absolute PREP/COMPACT/STOP gates one-to-one.

/// A context-budget band. Ordered by severity so a fleet view can sort or colour by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetLevel {
    /// < 30% used — headroom.
    Ok,
    /// ≥ 30% — prepare a handoff; finish the current atomic step.
    Prep,
    /// ≥ 50% — compact/restart after the current task; no new accumulation.
    Compact,
    /// ≥ 80% — stop taking new work; compact now.
    Stop,
    /// ≥ 90% — context-wall risk; compact immediately, even mid-thought.
    Emergency,
}

impl BudgetLevel {
    /// Classify a context-USED percentage into its band (token-budget canon: 30/50/80/90).
    pub fn classify(used_pct: u8) -> Self {
        match used_pct {
            p if p >= 90 => Self::Emergency,
            p if p >= 80 => Self::Stop,
            p if p >= 50 => Self::Compact,
            p if p >= 30 => Self::Prep,
            _ => Self::Ok,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Prep => "PREP",
            Self::Compact => "COMPACT",
            Self::Stop => "STOP",
            Self::Emergency => "EMERGENCY",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_every_band_boundary_per_canon() {
        // Pin each threshold edge — a quiet off-by-one here mis-reports the whole fleet.
        assert_eq!(BudgetLevel::classify(0), BudgetLevel::Ok);
        assert_eq!(BudgetLevel::classify(29), BudgetLevel::Ok);
        assert_eq!(BudgetLevel::classify(30), BudgetLevel::Prep);
        assert_eq!(BudgetLevel::classify(49), BudgetLevel::Prep);
        assert_eq!(BudgetLevel::classify(50), BudgetLevel::Compact);
        assert_eq!(BudgetLevel::classify(79), BudgetLevel::Compact);
        assert_eq!(BudgetLevel::classify(80), BudgetLevel::Stop);
        assert_eq!(BudgetLevel::classify(89), BudgetLevel::Stop);
        assert_eq!(BudgetLevel::classify(90), BudgetLevel::Emergency);
        assert_eq!(BudgetLevel::classify(100), BudgetLevel::Emergency);
    }

    #[test]
    fn severity_orders_ascending() {
        assert!(BudgetLevel::Ok < BudgetLevel::Prep);
        assert!(BudgetLevel::Prep < BudgetLevel::Compact);
        assert!(BudgetLevel::Compact < BudgetLevel::Stop);
        assert!(BudgetLevel::Stop < BudgetLevel::Emergency);
    }
}
