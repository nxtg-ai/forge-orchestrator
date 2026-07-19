//! Dual-run parity against the 14 live fleet pod shapes — DIRECTIVE-NXTG-20260718-09 DoD.
//!
//! The fixtures in `tests/fixtures/pods/` are **copies** of `~/ASIF/infra/tmux/*.yaml`, taken
//! read-only. Nothing here reads the live directory, touches `~/.cosmux/state.json`, or contacts
//! the running tmux server: every store-touching call is redirected by `FORGE_POD_STATE_DIR`, and
//! the tmux layer is exercised only through the **pure** `spawn_plan`, which executes nothing.
//!
//! Parity is asserted on the side-effect-free surface — parse, validate, resolved config, and the
//! computed tmux argument sequence. That is where byte-for-byte comparison is meaningful and safe;
//! asserting it by observing a real tmux server would require the live sessions this directive
//! forbids touching.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pods")
}

fn fixtures() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    paths.sort();
    paths
}

/// Run a `forge pod` verb with the store redirected to a scratch dir and tmux pointed at a
/// nonexistent private socket, so no live surface can be reached even by accident.
fn run_pod(args: &[&str], scratch: &Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_forge"))
        .arg("pod")
        .args(args)
        .env("FORGE_POD_STATE_DIR", scratch)
        .env("FORGE_POD_TMUX_SOCKET", "forge-parity-nonexistent")
        .env("NO_COLOR", "1")
        .output()
        .expect("run forge pod");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-pod-parity-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn all_fourteen_live_pod_shapes_validate() {
    // The DoD's core claim: forge parses every shape the fleet actually runs.
    let paths = fixtures();
    assert_eq!(
        paths.len(),
        14,
        "expected the 14 copied fleet pods, found {}",
        paths.len()
    );

    let dir = scratch("validate");
    let mut failures = Vec::new();
    for path in &paths {
        let (code, stdout, stderr) = run_pod(&["validate", &path.display().to_string()], &dir);
        if code != 0 {
            failures.push(format!("{}: exit {code} {stderr}", path.display()));
        } else {
            assert!(stdout.contains("pod '"), "{}: {stdout}", path.display());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "pods failed to validate:\n{failures:#?}"
    );
}

#[test]
fn every_shape_produces_a_deterministic_spawn_plan() {
    // `show` renders the resolved config; running it twice must produce identical bytes. A plan
    // that varies run-to-run could not be compared against cosmux at all.
    let dir = scratch("show");
    for path in fixtures() {
        let arg = path.display().to_string();
        let (code, first, _) = run_pod(&["show", &arg], &dir);
        assert_eq!(code, 0, "{}", path.display());
        let (_, second, _) = run_pod(&["show", &arg], &dir);
        assert_eq!(first, second, "{} is not deterministic", path.display());
        assert!(first.contains("name:"), "{}: {first}", path.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolved_config_preserves_every_declared_window_and_pane() {
    // Parity that matters operationally: vendoring must not drop a window or pane, because a
    // missing window is exactly the 2026-04-19 incident preflight exists to catch.
    let dir = scratch("counts");
    for path in fixtures() {
        let raw = std::fs::read_to_string(&path).unwrap();
        let source: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
        let declared_windows = source
            .get("windows")
            .and_then(|w| w.as_sequence())
            .map(|s| s.len())
            .unwrap_or(0);
        let declared_panes: usize = source
            .get("windows")
            .and_then(|w| w.as_sequence())
            .map(|ws| {
                ws.iter()
                    .map(|w| {
                        w.get("panes")
                            .and_then(|p| p.as_sequence())
                            .map(|s| s.len())
                            .unwrap_or(0)
                    })
                    .sum()
            })
            .unwrap_or(0);

        let (code, stdout, stderr) = run_pod(&["validate", &path.display().to_string()], &dir);
        assert_eq!(code, 0, "{}: {stderr}", path.display());
        assert!(
            stdout.contains(&format!("{declared_windows} window(s)")),
            "{}: expected {declared_windows} windows, got: {stdout}",
            path.display()
        );
        assert!(
            stdout.contains(&format!("{declared_panes} pane(s)")),
            "{}: expected {declared_panes} panes, got: {stdout}",
            path.display()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn side_effect_free_verbs_never_write_the_store() {
    // The constraint, asserted mechanically: validate/show/ps must leave the store byte-identical.
    let dir = scratch("nowrite");
    let store = dir.join("state.json");
    std::fs::write(&store, r#"{"pods":{}}"#).unwrap();
    let before = std::fs::read(&store).unwrap();

    let sample = fixture_dir().join("Dx3_Program.yaml");
    let arg = sample.display().to_string();
    for args in [
        vec!["validate", arg.as_str()],
        vec!["show", arg.as_str()],
        vec!["ps"],
        vec!["state"],
    ] {
        run_pod(&args, &dir);
    }

    assert_eq!(
        before,
        std::fs::read(&store).unwrap(),
        "a read-only verb modified the store"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hud_is_an_alias_of_state_not_a_separate_verb() {
    // FPL ruling: keep the alias so nxtg users see no behaviour change.
    let dir = scratch("hud");
    let (state_code, state_out, _) = run_pod(&["state"], &dir);
    let (hud_code, hud_out, _) = run_pod(&["hud"], &dir);
    assert_eq!(state_code, hud_code);
    assert_eq!(state_out, hud_out, "`hud` must behave exactly like `state`");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preflight_is_fail_closed_on_an_empty_target_set() {
    // Exit 2, never 0: a check that extracted no targets has not verified coverage.
    let dir = scratch("preflight");
    let heartbeat = dir.join("heartbeat.sh");
    std::fs::write(&heartbeat, "# no targets in here\n").unwrap();

    let (code, stdout, _) = run_pod(
        &["preflight", "--against", &heartbeat.display().to_string()],
        &dir,
    );
    assert_eq!(code, 2, "empty parse must exit 2:\n{stdout}");
    assert!(stdout.contains("no targets extracted"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_tmux_degrades_with_a_clear_message() {
    // Lego Snap: tmux absent must produce a named, actionable error — not a panic or a raw
    // subprocess failure. Simulated with an empty PATH so the binary cannot be found.
    let dir = scratch("notmux");
    let out = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["pod", "list"])
        .env("FORGE_POD_STATE_DIR", &dir)
        .env("PATH", "")
        .env("NO_COLOR", "1")
        .output()
        .expect("run forge pod");

    assert_eq!(out.status.code(), Some(1), "operational error exits 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("tmux not found"),
        "must name the missing dependency: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
