/// Doctor aggregation — one verdict from quality, release-debt, and drift dimensions.
pub mod doctor;
/// Append-only event log (`.forge/events.jsonl`) for audit trails.
pub mod event;
/// UAT finding capture and severity classification.
pub mod finding;
/// Five-dimension governance health score and drift detection.
pub mod governance;
/// Knowledge base — research, decisions, learnings, and patterns.
pub mod knowledge;
/// Master plan parsing and generation (`.forge/plan.md` / `plan.yaml`).
pub mod plan;
/// Quality gate checks for release readiness.
pub mod quality_gate;
/// Release-debt evaluation — version-surface agreement, lockfile sync, tag drift.
pub mod release_debt;
/// Ship phase — changelog, archive, and clean state for the next cycle.
pub mod ship;
/// Project state management (`.forge/state.json`).
pub mod state;
/// Task lifecycle — creation, assignment, status transitions, and file locking.
pub mod task;
