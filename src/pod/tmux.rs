//! tmux driver — vendored from cosmux v0.4.2 `tmux.rs`.
//!
//! # The test-isolation seam
//!
//! Upstream calls `Command::new("tmux")` directly, so any test would drive the operator's **live**
//! tmux server and could create or kill real fleet sessions. Every invocation here routes through
//! [`tmux_command`], which honours `FORGE_POD_TMUX_SOCKET` and prepends `-L <socket>` — the same
//! shape as the store seam in [`super::state`]: production behaviour is unchanged (no `-L`, the
//! default server), and tests point at a private throwaway server.
//!
//! The command **construction** is pure and separately tested ([`spawn_plan`]), so the argument
//! sequence for the 14 live pod shapes is verifiable without any tmux server at all.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::config::{Pane, PodConfig, Window, expand_path};
use super::error::{PodError, Result};

/// Points tmux at a private server. Test isolation only — unset in production.
pub const TMUX_SOCKET_ENV: &str = "FORGE_POD_TMUX_SOCKET";

/// Build a `tmux` command carrying the socket override when one is set.
fn tmux_command() -> Command {
    let mut cmd = Command::new("tmux");
    if let Some(socket) = std::env::var_os(TMUX_SOCKET_ENV)
        && !socket.is_empty()
    {
        cmd.arg("-L").arg(socket);
    }
    cmd
}

/// Tri-state result of a session-existence probe. Only [`Existence::ConfirmedAbsent`] is a benign
/// "nothing to recover"; [`Existence::Error`] means the server was unreachable and must surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existence {
    Present,
    ConfirmedAbsent,
    Error(String),
}

pub struct Tmux;

impl Tmux {
    /// Probe for tmux. A missing binary is a *clear degradation*, not a panic — the Lego Snap
    /// rule: forge's other commands keep working when tmux is absent.
    pub fn ensure_installed() -> Result<()> {
        match tmux_command().arg("-V").output() {
            Ok(out) if out.status.success() => Ok(()),
            _ => Err(PodError::TmuxNotFound),
        }
    }

    pub fn is_installed() -> bool {
        Self::ensure_installed().is_ok()
    }

    pub fn run(args: &[&str]) -> Result<Output> {
        tracing::debug!("tmux {}", args.join(" "));
        let out = tmux_command()
            .args(args)
            .output()
            .map_err(|e| PodError::TmuxFailed {
                cmd: format!("tmux {}", args.join(" ")),
                code: -1,
                stderr: e.to_string(),
            })?;
        if !out.status.success() {
            return Err(PodError::TmuxFailed {
                cmd: format!("tmux {}", args.join(" ")),
                code: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        Ok(out)
    }

    /// List a session's panes for recovery — **socket-aware and exit-status validated**, because it
    /// routes through [`Tmux::run`] (which prepends `-L <socket>` and returns `Err` on a non-zero
    /// tmux exit). An earlier version shelled out to `tmux` directly: it ignored
    /// `FORGE_POD_TMUX_SOCKET` (hitting the operator's real server) and swallowed the exit status, so
    /// a failed enumeration looked like "no dead panes". Format: `window|pane_index|pane_dead`.
    pub fn list_panes_for(session: &str) -> Result<String> {
        let out = Self::run(&[
            "list-panes",
            "-t",
            session,
            "-s",
            "-F",
            "#{window_name}|#{pane_index}|#{pane_dead}",
        ])?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn session_exists(name: &str) -> bool {
        tmux_command()
            .args(["has-session", "-t", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Tri-state session existence — the recovery guard must distinguish "server up, session gone"
    /// (benign) from "cannot reach the server" (an error to surface). `session_exists` collapses
    /// BOTH to `false`, so a dead/unreachable server there reads as benign absence and a recovery
    /// silently no-ops (regate round-2 A). `has-session` exits 1 either way; only the stderr
    /// distinguishes them: `can't find session` ⇒ absent; `no server running` / `error connecting`
    /// (or a spawn failure) ⇒ error.
    pub fn session_existence(name: &str) -> Existence {
        match tmux_command().args(["has-session", "-t", name]).output() {
            Ok(out) if out.status.success() => Existence::Present,
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.to_lowercase().contains("can't find session") {
                    Existence::ConfirmedAbsent
                } else {
                    Existence::Error(stderr.trim().to_string())
                }
            }
            Err(e) => Existence::Error(e.to_string()),
        }
    }

    pub fn list_sessions() -> Result<Vec<String>> {
        let out = tmux_command()
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()?;
        if !out.status.success() {
            // No server running is not an error — it means no sessions.
            return Ok(Vec::new());
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect())
    }

    pub fn kill_session(name: &str) -> Result<()> {
        if !Self::session_exists(name) {
            return Ok(());
        }
        Self::run(&["kill-session", "-t", name])?;
        Ok(())
    }
}

/// One tmux invocation, as arguments. The unit of the pure spawn plan.
pub type TmuxArgs = Vec<String>;

/// Compute the full tmux argument sequence for spawning a pod — **pure, no execution**.
///
/// This is the seam that makes spawn behaviour testable against the live pod shapes without a
/// tmux server: the plan for a given `PodConfig` is deterministic, so parity can be asserted on
/// the argument sequence rather than on observed side effects.
pub fn spawn_plan(pod: &PodConfig) -> Result<Vec<TmuxArgs>> {
    let mut plan: Vec<TmuxArgs> = Vec::new();
    let pod_root = pod.expanded_root();

    let first_window = pod
        .windows
        .first()
        .ok_or_else(|| PodError::InvalidConfig("no windows".into()))?;
    let first_pane = first_window
        .panes
        .first()
        .ok_or_else(|| PodError::InvalidConfig("no panes in first window".into()))?;

    let first_cwd = resolve_cwd(first_pane, pod_root.as_ref());
    plan.push(owned(&[
        "new-session",
        "-d",
        "-s",
        &pod.name,
        "-n",
        &first_window.name,
        "-c",
        &first_cwd.display().to_string(),
    ]));

    let win_target = format!("{}:{}", pod.name, first_window.name);
    if let Some(cmd) = first_pane.command.as_deref().filter(|c| !c.is_empty()) {
        plan.push(owned(&["send-keys", "-t", &win_target, cmd, "Enter"]));
    }

    for pane in first_window.panes.iter().skip(1) {
        let pane_cwd = resolve_cwd(pane, pod_root.as_ref());
        plan.push(owned(&[
            "split-window",
            "-t",
            &win_target,
            "-c",
            &pane_cwd.display().to_string(),
        ]));
        if let Some(cmd) = pane.command.as_deref().filter(|c| !c.is_empty()) {
            plan.push(owned(&["send-keys", "-t", &win_target, cmd, "Enter"]));
        }
    }

    if first_window.panes.len() > 1 {
        plan.push(owned(&[
            "select-layout",
            "-t",
            &win_target,
            &first_window.layout,
        ]));
    }

    for window in pod.windows.iter().skip(1) {
        plan.extend(window_plan(pod, window, pod_root.as_ref())?);
    }

    Ok(plan)
}

fn window_plan(
    pod: &PodConfig,
    window: &Window,
    pod_root: Option<&PathBuf>,
) -> Result<Vec<TmuxArgs>> {
    let mut plan = Vec::new();
    let first_pane = window
        .panes
        .first()
        .ok_or_else(|| PodError::InvalidConfig(format!("window '{}' has no panes", window.name)))?;
    let first_cwd = resolve_cwd(first_pane, pod_root);
    let target_session = format!("{}:", pod.name);

    plan.push(owned(&[
        "new-window",
        "-t",
        &target_session,
        "-n",
        &window.name,
        "-c",
        &first_cwd.display().to_string(),
    ]));

    let win_target = format!("{}:{}", pod.name, window.name);
    if let Some(cmd) = first_pane.command.as_deref().filter(|c| !c.is_empty()) {
        plan.push(owned(&["send-keys", "-t", &win_target, cmd, "Enter"]));
    }

    for pane in window.panes.iter().skip(1) {
        let pane_cwd = resolve_cwd(pane, pod_root);
        plan.push(owned(&[
            "split-window",
            "-t",
            &win_target,
            "-c",
            &pane_cwd.display().to_string(),
        ]));
        if let Some(cmd) = pane.command.as_deref().filter(|c| !c.is_empty()) {
            plan.push(owned(&["send-keys", "-t", &win_target, cmd, "Enter"]));
        }
    }

    if window.panes.len() > 1 {
        plan.push(owned(&["select-layout", "-t", &win_target, &window.layout]));
    }

    Ok(plan)
}

fn owned(args: &[&str]) -> TmuxArgs {
    args.iter().map(|s| s.to_string()).collect()
}

pub struct PodSpawner<'a> {
    pub pod: &'a PodConfig,
    pub force: bool,
}

impl<'a> PodSpawner<'a> {
    pub fn new(pod: &'a PodConfig, force: bool) -> Self {
        Self { pod, force }
    }

    pub fn spawn(&self) -> Result<()> {
        Tmux::ensure_installed()?;

        if Tmux::session_exists(&self.pod.name) {
            if self.force {
                tracing::warn!("session '{}' exists — killing (force)", self.pod.name);
                Tmux::kill_session(&self.pod.name)?;
            } else {
                return Err(PodError::SessionExists(self.pod.name.clone()));
            }
        }

        for args in spawn_plan(self.pod)? {
            let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            Tmux::run(&borrowed)?;
        }

        self.install_session_hooks()?;
        Ok(())
    }

    /// Install `pane-died` / `client-detached` hooks on a pod **forge itself spawned**.
    ///
    /// This is not the held "hook-rebind" work: rebinding means re-pointing *existing live cosmux
    /// sessions* at forge, which is a migration action and remains unimplemented. Installing hooks
    /// on a session this process just created is ordinary `start` behaviour.
    fn install_session_hooks(&self) -> Result<()> {
        let session = &self.pod.name;
        let exe = current_exe();

        if !self.pod.on_pane_dead.is_empty() {
            // Best-effort: older tmux versions may not support the pane-died hook target.
            let _ = Tmux::run(&[
                "set-hook",
                "-t",
                session,
                "pane-died",
                &forge_hook(&exe, SessionHook::PaneDied, session),
            ]);
            // Keep dead panes around so the hook can observe them.
            let _ = Tmux::run(&["set-option", "-t", session, "remain-on-exit", "on"]);
        }

        if !self.pod.after_detach.is_empty() {
            let _ = Tmux::run(&[
                "set-hook",
                "-t",
                session,
                "client-detached",
                &forge_hook(&exe, SessionHook::ClientDetached, session),
            ]);
        }
        Ok(())
    }
}

/// The forge binary path, used verbatim inside the hooks it installs. Falls back to the bare name
/// so a hook is still meaningful if the exe path cannot be read.
pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("forge"))
}

/// Which tmux session hook a value belongs to. The two forge drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHook {
    PaneDied,
    ClientDetached,
}

impl SessionHook {
    /// The tmux option name.
    pub fn tmux_name(self) -> &'static str {
        match self {
            Self::PaneDied => "pane-died",
            Self::ClientDetached => "client-detached",
        }
    }

    /// The forge subcommand the hook invokes.
    fn subcommand(self) -> &'static str {
        match self {
            Self::PaneDied => "_pane-recover",
            Self::ClientDetached => "_after-detach",
        }
    }

    pub const ALL: [SessionHook; 2] = [SessionHook::PaneDied, SessionHook::ClientDetached];
}

/// Build the forge form of a session hook — `<exe> pod _pane-recover <session>`.
///
/// The `pod ` infix is the exact discriminator [`crate::pod::adopt::classify_hook`] keys on: cosmux
/// invokes `<exe> _pane-recover`, forge invokes `<exe> pod _pane-recover`, so ownership is readable
/// straight off the hook string with no separate marker.
pub fn forge_hook(exe: &Path, kind: SessionHook, session: &str) -> String {
    format!(
        "run-shell '{} pod {} {} >> /tmp/forge-pod-{}.log 2>&1'",
        exe.display(),
        kind.subcommand(),
        session,
        session
    )
}

impl Tmux {
    /// Read a session hook's value, `None` when the hook is unset.
    ///
    /// tmux prints `pane-died[0] <value>`; the value is everything after the first `] `. An unset
    /// hook makes `show-options` fail, which is reported as `None`, not an error.
    pub fn show_hook(session: &str, kind: SessionHook) -> Option<String> {
        let out = tmux_command()
            .args(["show-options", "-t", session, kind.tmux_name()])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let line = String::from_utf8_lossy(&out.stdout);
        let line = line.trim_end();
        if line.is_empty() {
            return None;
        }
        line.split_once("] ").map(|(_, value)| value.to_string())
    }

    /// Set a session hook to a literal value (passed as one argv element — no shell re-parse).
    pub fn set_hook(session: &str, kind: SessionHook, value: &str) -> Result<()> {
        Tmux::run(&["set-hook", "-t", session, kind.tmux_name(), value])?;
        Ok(())
    }

    /// Remove a session hook entirely.
    pub fn unset_hook(session: &str, kind: SessionHook) -> Result<()> {
        // `-u` on an already-unset hook is not an error in practice; ignore a benign failure.
        let _ = Tmux::run(&["set-hook", "-u", "-t", session, kind.tmux_name()]);
        Ok(())
    }
}

fn resolve_cwd(pane: &Pane, pod_root: Option<&PathBuf>) -> PathBuf {
    if let Some(cwd) = pane.cwd.as_deref() {
        return expand_path(cwd);
    }
    if let Some(root) = pod_root {
        return root.clone();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(yaml: &str) -> PodConfig {
        PodConfig::parse(yaml, "test").expect("valid pod")
    }

    #[test]
    fn single_pane_plan_has_no_split_or_layout() {
        let plan = spawn_plan(&pod(
            "name: p\nwindows:\n  - name: main\n    panes:\n      - cwd: /tmp\n        command: htop\n",
        ))
        .unwrap();
        assert_eq!(plan[0][0], "new-session");
        assert!(plan[0].contains(&"/tmp".to_string()));
        assert_eq!(plan[1][0], "send-keys");
        assert!(
            !plan.iter().any(|a| a[0] == "select-layout"),
            "a single pane needs no layout: {plan:?}"
        );
        assert!(!plan.iter().any(|a| a[0] == "split-window"));
    }

    #[test]
    fn multi_pane_window_splits_then_applies_layout() {
        let plan = spawn_plan(&pod(
            "name: p\nwindows:\n  - name: main\n    layout: even-horizontal\n    panes:\n      - command: a\n      - command: b\n",
        ))
        .unwrap();
        let kinds: Vec<&str> = plan.iter().map(|a| a[0].as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "new-session",
                "send-keys",
                "split-window",
                "send-keys",
                "select-layout"
            ]
        );
        assert!(
            plan.last()
                .unwrap()
                .contains(&"even-horizontal".to_string())
        );
    }

    #[test]
    fn additional_windows_use_new_window_not_new_session() {
        let plan = spawn_plan(&pod(
            "name: p\nwindows:\n  - name: one\n    panes:\n      - command: a\n  - name: two\n    panes:\n      - command: b\n",
        ))
        .unwrap();
        assert_eq!(
            plan.iter().filter(|a| a[0] == "new-session").count(),
            1,
            "exactly one session is created"
        );
        let new_window = plan
            .iter()
            .find(|a| a[0] == "new-window")
            .expect("window 2");
        assert!(new_window.contains(&"two".to_string()));
    }

    #[test]
    fn pane_cwd_wins_over_pod_root_and_root_fills_the_gap() {
        let plan = spawn_plan(&pod(
            "name: p\nroot: /pod/root\nwindows:\n  - name: w\n    panes:\n      - cwd: /explicit\n        command: a\n      - command: b\n",
        ))
        .unwrap();
        assert!(
            plan[0].contains(&"/explicit".to_string()),
            "explicit pane cwd wins: {:?}",
            plan[0]
        );
        let split = plan.iter().find(|a| a[0] == "split-window").unwrap();
        assert!(
            split.contains(&"/pod/root".to_string()),
            "pane without cwd falls back to pod root: {split:?}"
        );
    }

    #[test]
    fn empty_command_produces_no_send_keys() {
        let plan = spawn_plan(&pod(
            "name: p\nwindows:\n  - name: w\n    panes:\n      - command: ''\n",
        ))
        .unwrap();
        assert!(
            !plan.iter().any(|a| a[0] == "send-keys"),
            "an empty command must not be typed into the pane: {plan:?}"
        );
    }

    #[test]
    fn socket_env_is_read_at_invocation_time() {
        // Proves the seam exists and is off by default — production must use the default server.
        let previous = std::env::var_os(TMUX_SOCKET_ENV);
        unsafe { std::env::remove_var(TMUX_SOCKET_ENV) };
        let plain = format!("{:?}", tmux_command());
        unsafe { std::env::set_var(TMUX_SOCKET_ENV, "forge-test-socket") };
        let redirected = format!("{:?}", tmux_command());
        match previous {
            Some(v) => unsafe { std::env::set_var(TMUX_SOCKET_ENV, v) },
            None => unsafe { std::env::remove_var(TMUX_SOCKET_ENV) },
        }
        assert!(
            !plain.contains("-L"),
            "default server in production: {plain}"
        );
        assert!(
            redirected.contains("forge-test-socket"),
            "tests must reach a private server: {redirected}"
        );
    }
}
