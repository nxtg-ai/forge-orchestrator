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
//! Migration surfaces (`adopt`, the `cosmux` shim, hook rebinding) are deliberately **absent** —
//! held pending the migration-protocol verdict per DIRECTIVE-NXTG-20260718-09.

/// Pod YAML parsing, validation, and the legacy config search order.
pub mod config;
/// Error type shared across the pod modules.
pub mod error;
/// Pod state store (`~/.cosmux/state.json`) with a fail-closed test-isolation seam.
pub mod state;
/// Pane/window template merging from `~/.config/cosmux/templates/`.
pub mod templates;
