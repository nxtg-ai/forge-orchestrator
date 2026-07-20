//! Dead-pane recovery — vendored from cosmux v0.4.2 `recover.rs`, plus the `.forge/` synergy.
//!
//! Invoked by the tmux `pane-died` hook. It re-spawns the dead pane in place from the store's
//! record of the original cwd and command.
//!
//! # `.forge/` synergy (directive item 4)
//!
//! When a recovered pane carries a `task:` binding, recovery also **re-claims that task** and
//! appends a `PaneRecovered` event to `.forge/events.jsonl`. Per RFC-0001 §3.1 a new `EventType`
//! variant is a MINOR schema change, and §3.2.2/§3.2.3 are exactly what make that safe for older
//! readers: an unknown variant maps to a catch-all rather than erroring the batch.
//!
//! Recovery is **best-effort by design** — it runs from a tmux hook where nothing surfaces an
//! error to a human. A failure to log an event must never prevent the pane from being respawned.

use super::error::Result;
use super::hooks::{HookKind, run_hooks};
use super::state::{self, PaneState};
use super::tmux::Tmux;

/// One dead pane, resolved against the store record that describes how to rebuild it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeadPane {
    pub window: String,
    pub index: usize,
    pub cwd: String,
    pub command: String,
    pub task: Option<String>,
}

/// Parse `tmux list-panes` output into dead panes — **pure**, so the mapping is testable.
///
/// Expected line format: `#{window_name}|#{pane_index}|#{pane_dead}`.
///
/// tmux pane indices honour `base-index`, which is commonly 1, while the store records panes
/// 0-based in declaration order. Upstream's mapping (`index - 1`, clamped) is preserved verbatim
/// rather than "fixed": changing it would silently respawn a different pane's command than cosmux
/// does, which is precisely the parity this vendoring must hold.
pub fn parse_dead_panes(listing: &str, pod: &state::PodState) -> Vec<DeadPane> {
    let mut dead = Vec::new();
    for line in listing.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 3 || parts[2] != "1" {
            continue;
        }
        let window_name = parts[0];
        let Ok(pane_index) = parts[1].parse::<usize>() else {
            continue;
        };
        let Some(window) = pod.windows.iter().find(|w| w.name == window_name) else {
            continue;
        };
        if window.panes.is_empty() {
            continue;
        }
        let want = pane_index
            .saturating_sub(1)
            .min(window.panes.len().saturating_sub(1));
        let Some(record) = window.panes.get(want) else {
            continue;
        };
        let PaneState {
            cwd, command, task, ..
        } = record;
        dead.push(DeadPane {
            window: window_name.to_string(),
            index: pane_index,
            cwd: cwd.clone(),
            command: command.clone(),
            task: task.clone(),
        });
    }
    dead
}

/// The tmux `pane-died` hook entry point.
///
/// During an in-flight adoption forge holds no production authority yet, so respawning here would
/// act ahead of the single-writer cutover. Instead it records a **durable** recovery intent (§1.5
/// step 3, round-7 C3) that the adopt drain executes once the transition is safe. In every other
/// state it recovers inline, exactly as before.
///
/// The disposition is driven by [`journal::append_recovery`]'s **atomic locked outcome**, not by a
/// separate `authority()` pre-check. An earlier version read `authority()` and then appended in two
/// steps: if the terminal commit won the lock in between, the append returned `NotAdopting` and the
/// pane was neither deferred nor recovered — stranded (Codex regate-15 P1-3). Appending
/// unconditionally makes the state decision happen once, inside the lock: `NotAdopting` now means
/// "forge already holds authority (or never adopted)", so recover inline immediately.
pub fn pane_recover(session: &str) -> Result<()> {
    match disposition(&super::journal::append_recovery(session)?) {
        Disposition::Deferred => {
            tracing::info!("pane-recover: adoption in flight, deferred recovery for '{session}'");
            Ok(())
        }
        Disposition::RecoverInline => pane_recover_inline(session),
    }
}

/// What to do with a pane given the atomic append outcome — **pure**, so the strand-fix is pinned
/// without needing to reproduce the race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// A durable intent was recorded; the adopt drain will respawn it.
    Deferred,
    /// Forge holds authority (adopted) or never adopted — respawn now.
    RecoverInline,
}

fn disposition(outcome: &super::journal::AppendOutcome) -> Disposition {
    use super::journal::AppendOutcome;
    match outcome {
        AppendOutcome::Appended | AppendOutcome::AlreadyPresent(_) => Disposition::Deferred,
        // `NotAdopting` is returned atomically by `append_recovery` under the journal lock, so it
        // also covers the race the old two-step TOCTOU stranded: the terminal commit took the lock
        // between the (removed) authority read and the append. Either way, recover inline.
        AppendOutcome::NotAdopting => Disposition::RecoverInline,
    }
}

/// Respawn every dead pane in a session immediately. Used both in the normal (adopted / unadopted)
/// path and by the adopt drain once deferral is over.
pub fn pane_recover_inline(session: &str) -> Result<()> {
    let Some(pod_state) = state::pod(session)? else {
        tracing::warn!("pane-recover: no state for pod '{session}'");
        return Ok(());
    };

    // Tri-state existence: only a CONFIRMED absent session (server up, session gone) is benign.
    // An unreachable server is an ERROR that must surface — collapsing it to "gone" would let a
    // dead-server recovery silently no-op (regate round-2 A).
    match Tmux::session_existence(session) {
        super::tmux::Existence::ConfirmedAbsent => {
            tracing::warn!(
                "pane-recover: session '{session}' no longer exists; nothing to recover"
            );
            return Ok(());
        }
        super::tmux::Existence::Error(why) => {
            return Err(super::error::PodError::Other(anyhow::anyhow!(
                "pane-recover: cannot determine whether session '{session}' exists \
                 (tmux server unreachable): {why}"
            )));
        }
        super::tmux::Existence::Present => {}
    }

    // Route through the socket-aware, exit-validated adapter — never a raw `tmux` (which would
    // ignore FORGE_POD_TMUX_SOCKET and swallow the exit status).
    let listing = Tmux::list_panes_for(session)?;

    for pane in parse_dead_panes(&listing, &pod_state) {
        tracing::info!(
            "pane-recover: found dead pane {}.{}",
            pane.window,
            pane.index
        );
        let target = format!("{session}:{}.{}", pane.window, pane.index);
        let _ = Tmux::run(&["respawn-pane", "-k", "-t", &target, "-c", &pane.cwd]);
        if !pane.command.is_empty() {
            let _ = Tmux::run(&["send-keys", "-t", &target, &pane.command, "Enter"]);
        }

        if let Some(task_id) = &pane.task {
            // Best-effort: a failure here is logged, never allowed to abort recovery.
            if let Err(error) = record_task_recovery(session, task_id, &pane) {
                tracing::warn!("pane-recover: could not record recovery for {task_id}: {error}");
            }
        }
    }

    if !pod_state.on_pane_dead.is_empty() {
        run_hooks(HookKind::OnPaneDead, &pod_state.on_pane_dead, session)?;
    }
    Ok(())
}

/// Append a `PaneRecovered` event for a task-bound pane.
///
/// Writes to the `.forge/` directory of the pane's own working directory — the project the pane
/// was running in — rather than assuming a single global project.
fn record_task_recovery(session: &str, task_id: &str, pane: &DeadPane) -> anyhow::Result<()> {
    let forge_dir = std::path::Path::new(&pane.cwd).join(".forge");
    if !forge_dir.is_dir() {
        // The pane is not inside a forge project; the binding is inert, not an error.
        return Ok(());
    }

    let logger = crate::core::event::EventLogger::new(&forge_dir);
    let event = crate::core::event::ForgeEvent::new(
        crate::core::event::EventType::PaneRecovered,
        format!(
            "pane {}.{} in pod '{session}' died and was respawned; task re-claimed",
            pane.window, pane.index
        ),
    )
    .with_task(task_id)
    .with_metadata(serde_json::json!({
        "pod": session,
        "window": pane.window,
        "pane_index": pane.index,
        "command": pane.command,
    }));
    logger.log(&event)?;
    Ok(())
}

pub fn after_detach(session: &str) -> Result<()> {
    let Some(pod_state) = state::pod(session)? else {
        return Ok(());
    };
    if pod_state.after_detach.is_empty() {
        return Ok(());
    }
    run_hooks(HookKind::AfterDetach, &pod_state.after_detach, session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pod::state::{PodState, WindowState};

    fn pod_with(panes: Vec<PaneState>) -> PodState {
        PodState {
            status: "running".into(),
            started_at: "2026-07-18T00:00:00Z".into(),
            source_path: "/tmp/p.yaml".into(),
            windows: vec![WindowState {
                name: "main".into(),
                panes,
            }],
            on_pane_dead: vec![],
            after_detach: vec![],
        }
    }

    fn pane(cwd: &str, command: &str, task: Option<&str>) -> PaneState {
        PaneState {
            index: 0,
            cwd: cwd.into(),
            command: command.into(),
            task: task.map(String::from),
        }
    }

    #[test]
    fn only_panes_marked_dead_are_recovered() {
        let pod = pod_with(vec![pane("/a", "cmd-a", None)]);
        let dead = parse_dead_panes("main|1|0\nmain|1|1\n", &pod);
        assert_eq!(dead.len(), 1, "a live pane must not be respawned");
        assert_eq!(dead[0].command, "cmd-a");
    }

    #[test]
    fn tmux_base_index_maps_to_the_stored_declaration_order() {
        // tmux base-index is commonly 1; the store is 0-based. Upstream's index-1 mapping is
        // preserved deliberately so forge respawns the same command cosmux would.
        let pod = pod_with(vec![
            pane("/first", "cmd-first", None),
            pane("/second", "cmd-second", None),
        ]);
        let dead = parse_dead_panes("main|2|1\n", &pod);
        assert_eq!(dead[0].cwd, "/second");
        assert_eq!(dead[0].command, "cmd-second");
    }

    #[test]
    fn out_of_range_index_clamps_instead_of_panicking() {
        let pod = pod_with(vec![pane("/only", "cmd", None)]);
        let dead = parse_dead_panes("main|99|1\n", &pod);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].cwd, "/only");
    }

    #[test]
    fn unknown_window_and_malformed_lines_are_skipped() {
        let pod = pod_with(vec![pane("/a", "cmd", None)]);
        assert!(parse_dead_panes("ghost|1|1\n", &pod).is_empty());
        assert!(parse_dead_panes("garbage\n", &pod).is_empty());
        assert!(
            parse_dead_panes("main|notanumber|1\n", &pod).is_empty(),
            "a non-numeric index must be skipped, not panic"
        );
        assert!(parse_dead_panes("", &pod).is_empty());
    }

    #[test]
    fn disposition_routes_notadopting_to_inline_recovery() {
        use super::super::journal::AppendOutcome;
        use super::super::journal::StepState;
        // The strand-fix, pinned: the race outcome (`NotAdopting`, which journal.rs proves the
        // terminal commit produces when it wins the lock) must recover inline, never be dropped.
        assert_eq!(
            disposition(&AppendOutcome::NotAdopting),
            Disposition::RecoverInline
        );
        assert_eq!(disposition(&AppendOutcome::Appended), Disposition::Deferred);
        assert_eq!(
            disposition(&AppendOutcome::AlreadyPresent(StepState::Pending)),
            Disposition::Deferred
        );
    }

    #[test]
    fn task_binding_is_carried_through_recovery() {
        let pod = pod_with(vec![pane("/proj", "ccyolo", Some("T-042"))]);
        let dead = parse_dead_panes("main|1|1\n", &pod);
        assert_eq!(dead[0].task.as_deref(), Some("T-042"));
    }

    #[test]
    fn pane_without_a_task_binding_recovers_with_no_task() {
        let pod = pod_with(vec![pane("/proj", "htop", None)]);
        let dead = parse_dead_panes("main|1|1\n", &pod);
        assert_eq!(dead[0].task, None);
    }

    #[test]
    fn recording_into_a_non_forge_directory_is_inert_not_an_error() {
        let dir = std::env::temp_dir().join(format!("forge-pod-norecord-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = DeadPane {
            window: "main".into(),
            index: 1,
            cwd: dir.display().to_string(),
            command: "x".into(),
            task: Some("T-1".into()),
        };
        assert!(
            record_task_recovery("pod", "T-1", &p).is_ok(),
            "a pane outside a forge project must not error"
        );
        assert!(!dir.join(".forge").exists(), "and must not create .forge");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recording_appends_a_pane_recovered_event() {
        let dir = std::env::temp_dir().join(format!("forge-pod-record-{}", std::process::id()));
        let forge = dir.join(".forge");
        std::fs::create_dir_all(&forge).unwrap();
        let p = DeadPane {
            window: "main".into(),
            index: 1,
            cwd: dir.display().to_string(),
            command: "ccyolo".into(),
            task: Some("T-042".into()),
        };
        record_task_recovery("mypod", "T-042", &p).expect("record");

        let log = std::fs::read_to_string(forge.join("events.jsonl")).expect("events written");
        assert!(log.contains("pane_recovered"), "{log}");
        assert!(log.contains("T-042"), "{log}");
        assert!(log.contains("mypod"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
