//! `forge pod` — verb dispatch for the vendored cosmux surface.
//!
//! Parity with cosmux v0.4.2 is the contract: 11 public verbs plus 2 hidden recovery verbs, with
//! the same flags, output shape, and exit codes. The full matrix is documented in
//! `docs/pod-parity-matrix.md`.
//!
//! Note `hud` is an **alias of `state`**, not a distinct verb — cosmux registers it that way and
//! nxtg users type both. Losing the alias would be a silent behaviour change for them.

use std::path::Path;

use colored::Colorize;

use crate::pod::config::{PodConfig, resolve_pod_path};
use crate::pod::error::Result;
use crate::pod::state;
use crate::pod::templates;
use crate::pod::tmux::{PodSpawner, Tmux};
use crate::pod::{preflight, recover};

/// Exit code for "the check ran and found problems", distinct from an operational error (1).
const EXIT_UNCOVERED: i32 = 2;

/// Load a pod config by name or path, applying templates — the shared front half of most verbs.
fn load_pod(name_or_path: &str) -> Result<(PodConfig, std::path::PathBuf)> {
    let path = resolve_pod_path(name_or_path)?;
    let mut pod = PodConfig::load(&path)?;
    templates::apply_templates(&mut pod)?;
    Ok((pod, path))
}

/// A missing tmux binary degrades with a clear message rather than a panic or a stack trace.
///
/// The Lego Snap rule: forge's other surfaces keep working when tmux is absent, and the user is
/// told exactly what is missing instead of seeing a failed subprocess.
fn require_tmux() -> Result<()> {
    Tmux::ensure_installed()
}

pub fn start(name: &str, force: bool, attach: bool) -> Result<i32> {
    require_tmux()?;
    let (pod, path) = load_pod(name)?;

    crate::pod::hooks::run_hooks(
        crate::pod::hooks::HookKind::BeforeStart,
        &pod.before_start,
        &pod.name,
    )?;

    PodSpawner::new(&pod, force).spawn()?;
    state::record_spawn(&pod, &path)?;

    crate::pod::hooks::run_hooks(
        crate::pod::hooks::HookKind::AfterStart,
        &pod.after_start,
        &pod.name,
    )?;

    println!(
        "{} pod '{}' started ({} window(s))",
        "✓".green(),
        pod.name,
        pod.windows.len()
    );

    if attach {
        crate::pod::hooks::run_hooks(
            crate::pod::hooks::HookKind::BeforeAttach,
            &pod.before_attach,
            &pod.name,
        )?;
        println!("  attach with: tmux attach -t {}", pod.name);
    }
    Ok(0)
}

pub fn stop(name: &str) -> Result<i32> {
    require_tmux()?;
    Tmux::kill_session(name)?;
    state::record_stop(name)?;
    println!("{} pod '{name}' stopped", "✓".green());
    Ok(0)
}

pub fn list() -> Result<i32> {
    require_tmux()?;
    let sessions = Tmux::list_sessions()?;
    if sessions.is_empty() {
        println!("no tmux sessions");
        return Ok(0);
    }
    for s in sessions {
        println!("{s}");
    }
    Ok(0)
}

pub fn validate(name: &str) -> Result<i32> {
    let (pod, path) = load_pod(name)?;
    println!(
        "{} {} — pod '{}', {} window(s), {} pane(s)",
        "✓".green(),
        path.display(),
        pod.name,
        pod.windows.len(),
        pod.windows.iter().map(|w| w.panes.len()).sum::<usize>()
    );
    Ok(0)
}

pub fn show(name: &str) -> Result<i32> {
    let (pod, _) = load_pod(name)?;
    println!("{}", serde_yaml::to_string(&pod).unwrap_or_default());
    Ok(0)
}

/// `state` (alias `hud`) — print the store path and its contents.
pub fn show_state() -> Result<i32> {
    let path = state::state_path();
    println!("{}", path.display());
    let store = state::load()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&store).unwrap_or_default()
    );
    Ok(0)
}

/// `ps` — pods this tool manages, as opposed to `list` which shows every tmux session.
pub fn ps() -> Result<i32> {
    let store = state::load()?;
    if store.pods.is_empty() {
        println!("no pods recorded in {}", state::state_path().display());
        return Ok(0);
    }
    for (name, pod) in &store.pods {
        let live = Tmux::session_exists(name);
        println!(
            "{:<24} {:<10} started={} windows={} {}",
            name,
            pod.status,
            pod.started_at,
            pod.windows.len(),
            if live { "" } else { "(session gone)" }
        );
    }
    Ok(0)
}

/// `gc` — drop store entries whose tmux session no longer exists.
pub fn gc() -> Result<i32> {
    require_tmux()?;
    let mut store = state::load()?;
    let before = store.pods.len();
    let stale: Vec<String> = store
        .pods
        .keys()
        .filter(|name| !Tmux::session_exists(name))
        .cloned()
        .collect();
    for name in &stale {
        store.pods.remove(name);
        println!("{} dropped '{name}' (session gone)", "-".dimmed());
    }
    if stale.is_empty() {
        println!("nothing to collect ({before} pod(s) all live)");
    } else {
        state::save(&store)?;
    }
    Ok(0)
}

pub fn reload(name: &str, force: bool) -> Result<i32> {
    println!(
        "{} reload re-reads the YAML and restarts the session — any agent conversation context in \
         those panes is lost",
        "!".yellow()
    );
    stop(name)?;
    start(name, force, false)
}

pub fn preflight_cmd(pod: Option<&str>, against: Option<&Path>) -> Result<i32> {
    let report = preflight::run(pod, against)?;
    println!("heartbeat: {}", report.heartbeat_path.display());
    println!(
        "targets: {} | pods checked: {}",
        report.targets_found,
        report.pods_checked.len()
    );

    if report.empty_parse {
        // Fail-closed: a check that extracted nothing has not verified coverage.
        println!(
            "{} no targets extracted — cannot claim coverage. Check the heartbeat script format.",
            "FAIL".red().bold()
        );
        return Ok(EXIT_UNCOVERED);
    }

    for gap in &report.gaps {
        println!("{} {} — {}", "GAP".red(), gap.target.display(), gap.reason);
    }

    if report.gaps.is_empty() {
        println!(
            "{} all {} target(s) covered",
            "✓".green(),
            report.targets_found
        );
    } else {
        println!(
            "{} {} of {} target(s) uncovered",
            "FAIL".red().bold(),
            report.gaps.len(),
            report.targets_found
        );
    }
    Ok(report.exit_code())
}

pub fn completions(shell: clap_complete::Shell) -> Result<i32> {
    use clap::CommandFactory;
    let mut cmd = crate::cli::Cli::command();
    let bin = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(0)
}

pub fn pane_recover(session: &str) -> Result<i32> {
    recover::pane_recover(session)?;
    Ok(0)
}

pub fn after_detach(session: &str) -> Result<i32> {
    recover::after_detach(session)?;
    Ok(0)
}
