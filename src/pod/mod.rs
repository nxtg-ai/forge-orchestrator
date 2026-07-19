//! `forge pod` — declarative tmux pod management, vendored from cosmux v0.4.2 (Apache-2.0).
//!
//! Per the consolidation RFC's single-store protocol, this operates on the **existing** cosmux
//! store and pod locations in place: `~/.cosmux/state.json`, `~/.config/cosmux/templates/`, and the
//! legacy pod search order rooted at `~/ASIF/infra/tmux/`. There is no import step and no new
//! store path — a `forge pod` invocation and a `cosmux` invocation see the same files.
//!
//! Structure mirrors the split used elsewhere in this crate: pure parsing/merging/aggregation with
//! no IO ([`config`], [`templates`]), and IO confined to the modules that genuinely need it
//! ([`state`], and the tmux/hook layers). That is what lets the parity fixtures run without a tmux
//! server or a writable store.
//!
//! The migration protocol (consolidation RFC §1.5) is being built on top of that core. [`journal`]
//! is its foundation and its only authority signal: until the transition journal reads terminal
//! `adopted`, every forge pod-store write is refused and cosmux remains the sole production writer.
//! The surfaces that act on that authority — `adopt`, the `cosmux` shim, hook rebinding — land on
//! top of it, never beside it.

/// Pod YAML parsing, validation, and the legacy config search order.
pub mod config;
/// Error type shared across the pod modules.
pub mod error;
/// Lifecycle hooks (`before_start`, `on_pane_dead`, …).
pub mod hooks;
/// Adoption transition journal — the sole representation of production-write authority.
pub mod journal;
/// Heartbeat-coverage preflight — fail-closed on an empty target set.
pub mod preflight;
/// Dead-pane recovery, including `.forge/` task re-claim.
pub mod recover;
/// Pod state store (`~/.cosmux/state.json`) with a fail-closed test-isolation seam.
pub mod state;
/// Pane/window template merging from `~/.config/cosmux/templates/`.
pub mod templates;
/// tmux driver with a private-socket test seam, and the pure spawn plan.
pub mod tmux;

/// Serializes every test that mutates one of the pod isolation env vars.
///
/// Env vars are process-global and cargo runs tests in parallel threads, so the store, journal, and
/// tmux-socket seams must share **one** lock: two modules each holding their own would still race
/// each other, and the store's fail-closed guard would (correctly) panic the run.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
