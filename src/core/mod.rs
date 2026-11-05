//! The `.forge/` state API: [`state`] (project state and file locks), [`task`], [`event`] (the
//! append-only log), [`knowledge`] and [`plan`]. Modules hidden from these docs are internals.
/// Context-budget thresholds (PREP/COMPACT/STOP) behind the W2-C fleet HUD.
#[doc(hidden)]
pub mod budget;
/// Doctor aggregation — one verdict from quality, release-debt, and drift dimensions.
#[doc(hidden)]
pub mod doctor;
/// Append-only event log (`.forge/events.jsonl`) for audit trails.
#[warn(missing_docs)]
pub mod event;
/// UAT finding capture and severity classification.
#[doc(hidden)]
pub mod finding;
/// Five-dimension governance health score and drift detection.
#[doc(hidden)]
pub mod governance;
/// Knowledge base — research, decisions, learnings, and patterns.
#[warn(missing_docs)]
pub mod knowledge;
/// Master plan parsing and generation (`.forge/plan.md` / `plan.yaml`).
#[warn(missing_docs)]
pub mod plan;
/// Quality gate checks for release readiness.
#[doc(hidden)]
pub mod quality_gate;
/// Release-debt evaluation — version-surface agreement, lockfile sync, tag drift.
#[doc(hidden)]
pub mod release_debt;
/// Ship phase — changelog, archive, and clean state for the next cycle.
#[doc(hidden)]
pub mod ship;
/// Project state management (`.forge/state.json`).
#[warn(missing_docs)]
pub mod state;
/// Task lifecycle — creation, assignment, status transitions, and file locking.
#[warn(missing_docs)]
pub mod task;
