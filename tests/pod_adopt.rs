//! §1.5 adoption acceptance matrix — DIRECTIVE-NXTG-20260718-09.
//!
//! These drive the REAL binary against a REAL tmux server, but never the operator's: every
//! invocation redirects all four destructive seams to a temp/private location —
//! `FORGE_POD_TMUX_SOCKET` (a private `-L` server), `FORGE_POD_JOURNAL_DIR`, `FORGE_POD_STATE_DIR`,
//! and `FORGE_POD_SHIM_PATH`. The live `~/.cosmux/state.json`, the live tmux server, and
//! `~/.local/bin/cosmux` are unreachable by construction, not by care.
//!
//! Skips cleanly when tmux is unavailable.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A self-contained adoption world: private tmux socket + temp journal/store/shim, all cleaned up.
struct World {
    socket: String,
    root: PathBuf,
    journal_dir: PathBuf,
    store_dir: PathBuf,
    shim: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let socket = format!("forge-adopt-{tag}-{}", std::process::id());
        let root = std::env::temp_dir().join(format!("forge-adopt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let journal_dir = root.join("state");
        let store_dir = root.join("store");
        std::fs::create_dir_all(&journal_dir).unwrap();
        std::fs::create_dir_all(&store_dir).unwrap();
        let world = Self {
            socket,
            shim: root.join("bin").join("cosmux"),
            journal_dir,
            store_dir,
            root,
        };
        world.kill_server();
        world
    }

    fn tmux(&self, args: &[&str]) -> std::process::Output {
        Command::new("tmux")
            .arg("-L")
            .arg(&self.socket)
            .args(args)
            .output()
            .expect("tmux")
    }

    fn kill_server(&self) {
        let _ = self.tmux(&["kill-server"]);
    }

    /// Run `forge pod …` with every seam redirected into this world.
    fn forge(&self, args: &[&str]) -> (i32, String, String) {
        self.forge_env(args, &[])
    }

    fn forge_env(&self, args: &[&str], extra: &[(&str, &str)]) -> (i32, String, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
        cmd.arg("pod")
            .args(args)
            .env("FORGE_POD_TMUX_SOCKET", &self.socket)
            .env("FORGE_POD_JOURNAL_DIR", &self.journal_dir)
            .env("FORGE_POD_STATE_DIR", &self.store_dir)
            .env("FORGE_POD_SHIM_PATH", &self.shim)
            .env("NO_COLOR", "1");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run forge pod");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn make_session_with_cosmux_hooks(&self, name: &str, client_detached: bool) {
        self.tmux(&["new-session", "-d", "-s", name]);
        let pane_died = format!(
            "run-shell '/usr/bin/cosmux _pane-recover {name} >> /tmp/cosmux-{name}.log 2>&1'"
        );
        self.tmux(&["set-hook", "-t", name, "pane-died", &pane_died]);
        if client_detached {
            let detached = format!(
                "run-shell '/usr/bin/cosmux _after-detach {name} >> /tmp/cosmux-{name}.log 2>&1'"
            );
            self.tmux(&["set-hook", "-t", name, "client-detached", &detached]);
        }
    }

    fn make_foreign_session(&self, name: &str) {
        self.tmux(&["new-session", "-d", "-s", name]);
        self.tmux(&[
            "set-hook",
            "-t",
            name,
            "pane-died",
            "run-shell 'my-monitor.sh'",
        ]);
    }

    fn hook(&self, session: &str, name: &str) -> String {
        let out = self.tmux(&["show-options", "-t", session, name]);
        let line = String::from_utf8_lossy(&out.stdout);
        line.trim_end()
            .split_once("] ")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default()
    }

    fn journal_state(&self) -> Option<String> {
        let raw = std::fs::read_to_string(self.journal_dir.join("pod-adoption.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        v.get("state").and_then(|s| s.as_str()).map(String::from)
    }

    fn journal_exists(&self) -> bool {
        self.journal_dir.join("pod-adoption.json").exists()
    }

    fn recoveries(&self) -> std::collections::BTreeMap<String, String> {
        let raw =
            std::fs::read_to_string(self.journal_dir.join("pod-adoption.json")).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        v.get("steps")
            .and_then(|s| s.get("recoveries"))
            .and_then(|r| r.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shim_present_and_exec(&self) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&self.shim)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    fn unadopt_script(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/forge-pod-unadopt.sh")
    }

    /// Run the standalone script, optionally overriding the tmux socket (e.g. a dead one).
    fn run_script(&self, socket: &str) -> std::process::ExitStatus {
        Command::new("bash")
            .arg(self.unadopt_script())
            .env("FORGE_POD_TMUX_SOCKET", socket)
            .env("FORGE_POD_JOURNAL_DIR", &self.journal_dir)
            .env("FORGE_POD_SHIM_PATH", &self.shim)
            .status()
            .expect("run unadopt script")
    }

    fn backup_exists(&self) -> bool {
        self.journal_dir.join("pod-adoption.hooks").exists()
    }

    fn backup_path(&self) -> PathBuf {
        self.journal_dir.join("pod-adoption.hooks")
    }

    fn journal_path(&self) -> PathBuf {
        self.journal_dir.join("pod-adoption.json")
    }

    /// Write a pod record into the store so `state::pod` returns it and recovery actually
    /// enumerates (instead of early-returning). The pane binds a `.forge` `task:` and points at
    /// `pane_cwd`, so a recovery logs a `PaneRecovered` event there — the observable that proves
    /// enumeration reached the pane through the private socket.
    fn seed_pod_record(
        &self,
        name: &str,
        window: &str,
        pane_cwd: &Path,
        command: &str,
        task: &str,
    ) {
        let json = serde_json::json!({
            "pods": {
                name: {
                    "status": "running",
                    "started_at": "2026-07-19T00:00:00Z",
                    "source_path": format!("/tmp/{name}.yaml"),
                    "windows": [{
                        "name": window,
                        "panes": [{
                            "index": 0,
                            "cwd": pane_cwd.display().to_string(),
                            "command": command,
                            "task": task,
                        }]
                    }],
                    "on_pane_dead": [],
                    "after_detach": []
                }
            }
        });
        std::fs::write(
            self.store_dir.join("state.json"),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();
    }

    /// Create a session on the private socket with one pane that dies (remain-on-exit keeps it as a
    /// dead pane). Polls until tmux reports `pane_dead=1`.
    fn make_session_with_dead_pane(&self, name: &str, window: &str) {
        self.tmux(&["new-session", "-d", "-s", name, "-n", window]);
        self.tmux(&["set-option", "-t", name, "remain-on-exit", "on"]);
        // Kill the pane's process so it becomes a dead pane.
        self.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &format!("{name}:{window}"),
            "false",
        ]);
        for _ in 0..200 {
            let out = self.tmux(&["list-panes", "-t", name, "-s", "-F", "#{pane_dead}"]);
            if String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l == "1")
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.kill_server();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn adopt_rebinds_cosmux_hooks_installs_shim_and_reaches_terminal() {
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("full");
    w.make_session_with_cosmux_hooks("alpha", false);
    w.make_session_with_cosmux_hooks("beta", true);
    w.make_foreign_session("gamma");

    let (code, out, err) = w.forge(&["adopt"]);
    assert_eq!(code, 0, "adopt failed: {out}{err}");

    // cosmux hooks are now forge's; the foreign hook is untouched.
    assert!(
        w.hook("alpha", "pane-died").contains("pod _pane-recover"),
        "{}",
        w.hook("alpha", "pane-died")
    );
    assert!(w.hook("beta", "pane-died").contains("pod _pane-recover"));
    assert!(
        w.hook("beta", "client-detached")
            .contains("pod _after-detach")
    );
    // tmux may re-quote on readback, so assert the invariant, not the byte string: the foreign
    // hook still points at its own script and was never rebound to forge.
    let gamma = w.hook("gamma", "pane-died");
    assert!(
        gamma.contains("my-monitor.sh"),
        "foreign hook lost: {gamma}"
    );
    assert!(
        !gamma.contains("_pane-recover"),
        "a foreign hook must never be rebound: {gamma}"
    );

    assert!(
        w.shim_present_and_exec(),
        "shim must be installed and executable"
    );
    assert_eq!(w.journal_state().as_deref(), Some("adopted"));

    // Idempotent: a second adopt is a no-op success.
    let (code, out, _) = w.forge(&["adopt"]);
    assert_eq!(code, 0);
    assert!(out.contains("already adopted"), "{out}");
}

#[test]
fn standalone_script_restores_hooks_removes_shim_and_clears_the_journal() {
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("script");
    w.make_session_with_cosmux_hooks("alpha", true);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    assert!(w.hook("alpha", "pane-died").contains("pod _pane-recover"));

    // Roll back with the STANDALONE script — no forge binary involved.
    let status = Command::new("bash")
        .arg(w.unadopt_script())
        .env("FORGE_POD_TMUX_SOCKET", &w.socket)
        .env("FORGE_POD_JOURNAL_DIR", &w.journal_dir)
        .env("FORGE_POD_SHIM_PATH", &w.shim)
        .status()
        .expect("run unadopt script");
    assert!(status.success(), "standalone rollback failed");

    // The exact captured original is back — cosmux-owned, not forge.
    let restored = w.hook("alpha", "pane-died");
    assert!(
        restored.contains("/usr/bin/cosmux _pane-recover"),
        "{restored}"
    );
    assert!(!restored.contains("pod _pane-recover"), "{restored}");
    assert!(
        w.hook("alpha", "client-detached")
            .contains("/usr/bin/cosmux _after-detach")
    );
    assert!(!w.shim.exists(), "shim must be removed");
    assert!(!w.journal_exists(), "journal must be cleared → unadopted");
}

#[test]
fn forge_unadopt_is_equivalent_to_the_script() {
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("unadopt");
    w.make_session_with_cosmux_hooks("solo", false);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    let (code, out, err) = w.forge(&["unadopt"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        w.hook("solo", "pane-died")
            .contains("/usr/bin/cosmux _pane-recover")
    );
    assert!(!w.journal_exists());
    assert!(!w.shim.exists());
}

#[test]
fn a_crash_at_every_sub_point_resumes_to_a_single_writer() {
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    // One marker per (step × sub-point) the ledger distinguishes, plus the terminal boundary.
    let markers = [
        "shim/before-pending",
        "shim/after-pending",
        "shim/after-side-effect",
        "shim/after-complete",
        "hooks:node/before-pending",
        "hooks:node/after-pending",
        "hooks:node/after-side-effect",
        "hooks:node/after-complete",
        "terminal/before",
        "terminal/after",
    ];

    for marker in markers {
        let w = World::new(&format!("crash-{}", marker.replace(['/', ':'], "-")));
        w.make_session_with_cosmux_hooks("node", false);

        // Crash exactly at the marker. Disk is left in whatever durable state that implies.
        let (code, _out, _err) = w.forge_env(&["adopt"], &[("FORGE_POD_ADOPT_FAIL_AT", marker)]);
        assert_ne!(code, 0, "marker {marker}: injected crash must fail the run");
        // `terminal/after` fires AFTER the commit write, so the journal is legitimately `adopted`
        // by then; every earlier marker must not have committed.
        if marker != "terminal/after" {
            assert_ne!(
                w.journal_state().as_deref(),
                Some("adopted"),
                "marker {marker}: must not have committed"
            );
        }

        // Resume with no injection — must converge to a single authorized writer.
        let (code, out, err) = w.forge(&["adopt"]);
        assert_eq!(code, 0, "marker {marker}: resume failed: {out}{err}");
        assert_eq!(
            w.journal_state().as_deref(),
            Some("adopted"),
            "marker {marker}: resume did not reach terminal"
        );
        assert!(
            w.shim_present_and_exec(),
            "marker {marker}: shim missing after resume"
        );
        assert!(
            w.hook("node", "pane-died").contains("pod _pane-recover"),
            "marker {marker}: hook not rebound after resume"
        );
    }
}

#[test]
fn a_pane_death_during_adopting_defers_then_drains_on_resume() {
    // The round-7 C3 leg: a pane-died hook firing mid-adoption records a DURABLE recovery intent
    // (it has no authority yet), and the adopt drain executes it before the terminal commit. This
    // is the only test that runs `drain_recoveries` + the real deferring `_pane-recover` path.
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("drain");
    w.make_session_with_cosmux_hooks("node", false);

    // Stop at `adopting` with shim + hooks complete and no recoveries yet.
    let (code, _, _) = w.forge_env(
        &["adopt"],
        &[("FORGE_POD_ADOPT_FAIL_AT", "terminal/before")],
    );
    assert_ne!(code, 0);
    assert_eq!(w.journal_state().as_deref(), Some("adopting"));

    // A pane dies now (AFTER adopt cleared preflight — a dead pane before adopt would refuse it):
    // seed the store record + a `.forge` project so the drain ENUMERATES and logs a PaneRecovered
    // event, and kill a pane in the live private-socket session so there is a real dead pane to find.
    let proj = w.root.join("node-proj");
    std::fs::create_dir_all(proj.join(".forge")).unwrap();
    w.seed_pod_record("node", "main", &proj, "sleep 300", "T-1");
    // Recreate `node` with a `main` window carrying a dead pane (the earlier session had a different
    // window name); the store record's window/pane must match for enumeration to map it.
    w.tmux(&["kill-session", "-t", "node"]);
    w.make_session_with_dead_pane("node", "main");

    // The tmux hook runs `forge pod _pane-recover node`, which defers because authority is Adopting.
    let (code, out, err) = w.forge(&["_pane-recover", "node"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        w.recoveries().get("node").map(String::as_str),
        Some("pending"),
        "a deferred recovery must be durable in the journal: {:?}",
        w.recoveries()
    );

    // Resume: the drain executes the recovery, flips it complete, then commits.
    let (code, out, err) = w.forge(&["adopt"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(w.journal_state().as_deref(), Some("adopted"));
    assert_eq!(
        w.recoveries().get("node").map(String::as_str),
        Some("complete"),
        "the deferred recovery must complete before the terminal commit"
    );
    // The proof that enumeration actually ran through the PRIVATE socket (not the operator's server,
    // and not an early-return): the dead pane was recovered and logged its event.
    let events =
        std::fs::read_to_string(proj.join(".forge").join("events.jsonl")).unwrap_or_default();
    assert!(
        events.contains("pane_recovered"),
        "the drain must enumerate the dead pane via the private socket and recover it; \
         events.jsonl: {events:?}"
    );
}

#[test]
fn repair_rolls_a_corrupt_journal_back_to_unadopted() {
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("repair");
    w.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w.forge(&["adopt"]).0, 0);

    // Corrupt the journal → authority freezes into recovery.
    std::fs::write(w.journal_dir.join("pod-adoption.json"), "{ truncated").unwrap();
    let (code, out, _) = w.forge(&["adopt"]);
    assert_ne!(
        code, 0,
        "a corrupt journal must refuse a plain adopt: {out}"
    );

    // --repair reconciles deterministically back to unadopted.
    let (code, out, err) = w.forge(&["adopt", "--repair"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(!w.journal_exists(), "repair must clear the journal");
    assert!(!w.shim.exists(), "repair must remove the shim");
    assert!(
        w.hook("node", "pane-died").contains("/usr/bin/cosmux"),
        "repair must restore the cosmux hook"
    );
}

#[test]
fn standalone_script_fails_closed_on_a_dead_socket_preserving_everything() {
    // regate-15 P1-2: Codex reproduced success-with-zero-hooks-restored on a dead socket — the
    // `|| true` swallowed every set-hook failure, then the script deleted shim+backup+journal and
    // exited 0. The cure: a restore that cannot be verified preserves state and exits nonzero.
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("failclosed");
    w.make_session_with_cosmux_hooks("node", true);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    assert!(w.hook("node", "pane-died").contains("pod _pane-recover"));

    // Roll back pointed at a DEAD socket — the sessions are unreachable there.
    let dead = format!("forge-dead-{}-{}", std::process::id(), "nonexistent");
    let status = w.run_script(&dead);
    assert!(
        !status.success(),
        "a rollback that could not verify any hook must exit nonzero"
    );
    // Nothing may have been deleted — the real sessions still carry forge hooks.
    assert!(
        w.journal_exists(),
        "journal must be preserved on a failed restore"
    );
    assert!(w.backup_exists(), "hook backup must be preserved");
    assert!(w.shim.exists(), "shim must be preserved");
    // And the journal must be frozen (`unadopting`), never left `adopted` (P1-1).
    assert_eq!(w.journal_state().as_deref(), Some("unadopting"));

    // The real session is untouched — still forge-owned, so a retry against the LIVE socket works.
    assert!(w.hook("node", "pane-died").contains("pod _pane-recover"));
    let status = w.run_script(&w.socket);
    assert!(
        status.success(),
        "retry against the live socket must complete"
    );
    assert!(!w.journal_exists());
    assert!(w.hook("node", "pane-died").contains("/usr/bin/cosmux"));
}

#[test]
fn script_fails_closed_when_the_backup_is_absent_or_truncated_for_recorded_hooks() {
    // regate-15 P1-4: Codex reproduced exit=0 with journal+shim gone while the live pane-died hook
    // was still `forge pod _pane-recover`, because an absent/truncated backup was read as
    // zero-hooks-to-restore. The journal's steps.hooks records what was rebound; every recorded
    // session must verify non-forge by live readback before anything is deleted.
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }

    // (a) ABSENT backup.
    let w = World::new("nobackup");
    w.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    assert!(w.hook("node", "pane-died").contains("pod _pane-recover"));
    std::fs::remove_file(w.backup_path()).unwrap(); // the backup is gone
    let status = w.run_script(&w.socket);
    assert!(
        !status.success(),
        "an absent backup for a recorded hook must fail closed, not claim success"
    );
    assert!(w.journal_exists(), "journal preserved");
    assert!(w.shim.exists(), "shim preserved");
    assert!(
        w.hook("node", "pane-died").contains("pod _pane-recover"),
        "the live hook is still forge-owned — the script must not have claimed success"
    );

    // (b) TRUNCATED backup — recorded `node` missing from the TSV.
    let w2 = World::new("truncbackup");
    w2.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w2.forge(&["adopt"]).0, 0);
    std::fs::write(w2.backup_path(), "").unwrap(); // truncated to empty
    let status = w2.run_script(&w2.socket);
    assert!(!status.success(), "a truncated backup must fail closed");
    assert!(w2.journal_exists());
    assert!(w2.shim.exists());
    assert!(w2.hook("node", "pane-died").contains("pod _pane-recover"));

    // (c) TRUNCATED journal — cannot know what was rebound → fail closed, don't touch it.
    let w3 = World::new("truncjournal");
    w3.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w3.forge(&["adopt"]).0, 0);
    std::fs::write(w3.journal_path(), "{ \"schema\": 1, \"sta").unwrap();
    let status = w3.run_script(&w3.socket);
    assert!(!status.success(), "a truncated journal must fail closed");
    assert!(
        w3.journal_exists(),
        "a truncated journal must be preserved, not deleted"
    );
    assert!(w3.shim.exists());

    // (d) LATE-truncated journal (regate-15 round-2 P1): the Codex shape — all tokens present, a
    // recorded session extractable, but the final top-level brace is MISSING. A grep/awk gate
    // accepted this and deleted a malformed authority journal; real JSON parsing must reject it.
    let w4 = World::new("latetrunc");
    w4.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w4.forge(&["adopt"]).0, 0);
    let good = std::fs::read_to_string(w4.journal_path()).unwrap();
    let malformed = good.trim_end().trim_end_matches('}'); // drop the closing brace, keep the tokens
    assert!(
        malformed.contains("\"state\"")
            && malformed.contains("\"hooks\"")
            && malformed.contains("node")
    );
    std::fs::write(w4.journal_path(), malformed).unwrap();
    let status = w4.run_script(&w4.socket);
    assert!(
        !status.success(),
        "a late-truncated (missing-final-brace) journal must fail JSON validation, not delete"
    );
    assert!(
        w4.journal_exists(),
        "malformed authority journal must be preserved"
    );
    assert!(w4.shim.exists(), "shim must be preserved");
    assert!(
        w4.hook("node", "pane-died").contains("pod _pane-recover"),
        "the live hook must be untouched — no false success"
    );
}

#[test]
fn freeze_takes_effect_before_any_set_hook_on_split_line_valid_json() {
    // regate-15 round-3 P1: the last sed shortcut. A VALID journal whose `state` key and value
    // straddle two lines passed JSON validation but defeated the line-oriented sed freeze, so hooks
    // rolled back while the journal still read `adopted` (dual-writer). The freeze now uses the same
    // json parser; this asserts the OBSERVED state at the moment `tmux set-hook` runs is
    // `unadopting`, never `adopted`. Uses a fake tmux (Codex's instrument) to capture that moment.
    let root = std::env::temp_dir().join(format!("forge-freeze-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let state_dir = root.join("state");
    let bin = root.join("bin");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let journal = state_dir.join("pod-adoption.json");
    let observed = state_dir.join("observed-state.txt");
    let shim = root.join("cosmux");
    std::fs::write(&shim, "shim\n").unwrap();

    // Valid schema-v1 JSON with `state` split across two lines.
    std::fs::write(
        &journal,
        "{\n  \"schema\": 1,\n  \"state\":\n    \"adopted\",\n  \"steps\": {\n    \"shim\": \"complete\",\n    \"hooks\": {\n      \"node\": \"complete\"\n    },\n    \"recoveries\": {}\n  },\n  \"ts\": \"t\",\n  \"by\": \"forge\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        state_dir.join("pod-adoption.hooks"),
        "node\tpane-died\t/usr/bin/cosmux _pane-recover node\n",
    )
    .unwrap();

    // Fake tmux: on set-hook it records the journal's CURRENT state (the observed moment);
    // has-session succeeds; show-options prints a cosmux (non-forge) hook so the restore "verifies".
    let fake = bin.join("tmux");
    std::fs::write(
        &fake,
        format!(
            "#!/usr/bin/env bash\ncase \" $* \" in\n  *' set-hook '*) python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"state\"])' '{}' >> '{}' 2>/dev/null; exit 0 ;;\n  *' has-session '*) exit 0 ;;\n  *' show-options '*) printf '%s\\n' 'pane-died /usr/bin/cosmux _pane-recover node'; exit 0 ;;\nesac\nexit 0\n",
            journal.display(),
            observed.display()
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&fake).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake, perms).unwrap();

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/forge-pod-unadopt.sh");
    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("bash")
        .arg(&script)
        .env("PATH", path_env)
        .env("FORGE_POD_JOURNAL_DIR", &state_dir)
        .env("FORGE_POD_SHIM_PATH", &shim)
        .env("FORGE_POD_TMUX_SOCKET", "fake")
        .status()
        .expect("run script");

    let observed_state = std::fs::read_to_string(&observed).unwrap_or_default();
    assert!(
        observed_state.contains("unadopting"),
        "the journal must read `unadopting` when set-hook runs, got: {observed_state:?}"
    );
    assert!(
        !observed_state.contains("adopted"),
        "authority must be frozen before any hook is touched — no `adopted` during set-hook"
    );
    assert!(status.success(), "a valid journal must still roll back");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn repair_freezes_authority_before_restoring_hooks() {
    // regate-15 P1-1: `--repair` on an `adopted` journal must write `unadopting` (freeze) BEFORE
    // restoring cosmux hooks — otherwise, since store-write authority is a lock-free journal read,
    // there is a window where cosmux hooks are back while forge is still authorized (dual-writer).
    // Observed via a repair whose restore fails (dead socket): the journal must already be
    // `unadopting`, not still `adopted`.
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let w = World::new("freeze");
    w.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    assert_eq!(w.journal_state().as_deref(), Some("adopted"));

    // Kill the private server so the hook restore inside repair fails.
    w.kill_server();
    let (code, _out, _err) = w.forge(&["adopt", "--repair"]);
    assert_ne!(code, 0, "repair with an unreachable server must fail");
    assert_eq!(
        w.journal_state().as_deref(),
        Some("unadopting"),
        "the journal must be frozen BEFORE the restore is attempted, never left `adopted`"
    );
}

#[test]
fn the_standalone_script_blocks_until_a_held_lock_is_released() {
    // Closes the acceptance leg checkpoint 4 deliberately refused to claim: the rollback script must
    // BLOCK on a lock the binary (here, a stand-in flock holder) is holding, not race it.
    if !tmux_available() {
        eprintln!("SKIP: tmux unavailable");
        return;
    }
    let has_flock = Command::new("flock")
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_flock {
        eprintln!("SKIP: flock(1) unavailable");
        return;
    }

    let w = World::new("block");
    w.make_session_with_cosmux_hooks("node", false);
    assert_eq!(w.forge(&["adopt"]).0, 0);
    assert!(w.journal_exists());

    let lock = w.journal_dir.join("pod-adoption.lock");
    // A stand-in holder keeps the lock for ~2s, exactly as an in-flight binary operation would.
    let mut holder = Command::new("flock")
        .arg("-x")
        .arg(&lock)
        .args(["-c", "sleep 2"])
        .spawn()
        .expect("spawn flock holder");
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Launch the rollback script while the lock is held; it must not finish yet.
    let mut script = Command::new("bash")
        .arg(w.unadopt_script())
        .env("FORGE_POD_TMUX_SOCKET", &w.socket)
        .env("FORGE_POD_JOURNAL_DIR", &w.journal_dir)
        .env("FORGE_POD_SHIM_PATH", &w.shim)
        .spawn()
        .expect("spawn unadopt script");

    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        script.try_wait().unwrap().is_none(),
        "the script must still be BLOCKED on the held lock"
    );
    assert!(
        w.journal_exists(),
        "blocked script must not have removed the journal yet"
    );

    // Once the holder releases, the script proceeds and completes the rollback.
    holder.wait().unwrap();
    let status = script.wait().unwrap();
    assert!(status.success(), "script must succeed after the lock frees");
    assert!(!w.journal_exists(), "journal cleared once the script ran");
}
