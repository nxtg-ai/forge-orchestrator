//! Lifecycle hooks — vendored from cosmux v0.4.2 `hooks.rs`.
//!
//! `log::*` is replaced by `tracing::*`; message text and the abort semantics are preserved.
//! Only `before_start` aborts on failure — every other hook logs and continues, because a failing
//! teardown hook must not strand a pod.

use std::process::Command;

use super::error::{PodError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    BeforeStart,
    AfterStart,
    BeforeAttach,
    AfterDetach,
    OnPaneDead,
}

impl HookKind {
    pub fn name(&self) -> &'static str {
        match self {
            HookKind::BeforeStart => "before_start",
            HookKind::AfterStart => "after_start",
            HookKind::BeforeAttach => "before_attach",
            HookKind::AfterDetach => "after_detach",
            HookKind::OnPaneDead => "on_pane_dead",
        }
    }

    /// Only a failing `before_start` aborts the operation.
    ///
    /// The asymmetry is deliberate: refusing to start is recoverable, but aborting midway through
    /// teardown leaves a half-stopped pod nobody owns.
    pub fn fail_aborts(&self) -> bool {
        matches!(self, HookKind::BeforeStart)
    }
}

pub fn run_hooks(kind: HookKind, commands: &[String], pod_name: &str) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }
    tracing::info!(
        "[{pod_name}] running {} hook(s) for {}",
        commands.len(),
        kind.name()
    );
    for cmd in commands {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            continue;
        }
        tracing::debug!("[{pod_name}] {}: {trimmed}", kind.name());
        match Command::new("sh").arg("-c").arg(trimmed).status() {
            Ok(s) if s.success() => continue,
            Ok(s) => {
                let msg = format!("'{trimmed}' exited with code {}", s.code().unwrap_or(-1));
                if kind.fail_aborts() {
                    return Err(PodError::HookFailed {
                        hook: kind.name().into(),
                        reason: msg,
                    });
                }
                tracing::warn!("[{pod_name}] {} non-fatal failure: {msg}", kind.name());
            }
            Err(e) => {
                let msg = format!("failed to spawn '{trimmed}': {e}");
                if kind.fail_aborts() {
                    return Err(PodError::HookFailed {
                        hook: kind.name().into(),
                        reason: msg,
                    });
                }
                tracing::warn!("[{pod_name}] {} spawn error: {msg}", kind.name());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_before_start_aborts() {
        assert!(HookKind::BeforeStart.fail_aborts());
        for kind in [
            HookKind::AfterStart,
            HookKind::BeforeAttach,
            HookKind::AfterDetach,
            HookKind::OnPaneDead,
        ] {
            assert!(!kind.fail_aborts(), "{} must not abort", kind.name());
        }
    }

    #[test]
    fn hook_names_match_the_yaml_keys() {
        assert_eq!(HookKind::BeforeStart.name(), "before_start");
        assert_eq!(HookKind::OnPaneDead.name(), "on_pane_dead");
        assert_eq!(HookKind::AfterDetach.name(), "after_detach");
    }

    #[test]
    fn empty_hook_list_is_a_no_op() {
        assert!(run_hooks(HookKind::BeforeStart, &[], "p").is_ok());
    }

    #[test]
    fn failing_before_start_aborts_with_the_hook_named() {
        let err = run_hooks(HookKind::BeforeStart, &["exit 3".into()], "p").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("before_start"), "{text}");
        assert!(text.contains("exit 3"), "{text}");
    }

    #[test]
    fn failing_teardown_hook_does_not_abort() {
        // A pod must still stop even when its after_detach hook is broken.
        assert!(run_hooks(HookKind::AfterDetach, &["exit 1".into()], "p").is_ok());
    }

    #[test]
    fn blank_commands_are_skipped() {
        assert!(run_hooks(HookKind::BeforeStart, &["   ".into()], "p").is_ok());
    }
}
