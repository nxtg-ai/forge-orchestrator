//! Per-invocation project-binding isolation — DIRECTIVE-NXTG-20260718-16.
//!
//! The finding: while a forge-ui server ran, `forge mcp --project <fixture>` returned forge-ui's
//! health instead of the fixture's, because the server-global `current_project` was overridable by
//! any consumer's `forge_set_project`, and `FORGE_PROJECT_ROOT` (the plugin's per-connection
//! binding) was ignored. These tests are the -14 instrument as a regression: an explicitly bound
//! server must read its OWN project regardless of another consumer's global switch, while an unbound
//! server keeps the single-project switching UX.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_forge")
}

/// Scaffold a project and seed a known health profile. `healthy` → SPEC + README (100/100);
/// otherwise bare (90/100, 2 warnings) — a deterministic discriminator between two consumers.
fn make_project(tag: &str, healthy: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-bind-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ok = Command::new(bin())
        .args(["--project", &dir.display().to_string(), "init"])
        .output()
        .expect("forge init");
    assert!(ok.status.success(), "init failed: {ok:?}");
    if healthy {
        std::fs::write(dir.join("SPEC.md"), "# Spec\nVision: v.\n").unwrap();
        std::fs::write(dir.join("README.md"), "# Readme\n").unwrap();
    } else {
        let _ = std::fs::remove_file(dir.join("SPEC.md"));
        let _ = std::fs::remove_file(dir.join("README.md"));
    }
    dir
}

/// Drive a `forge mcp` server: `cwd` is the process directory, `env` the extra vars, `calls` the
/// tool-call request lines. Returns the health summary from the LAST `forge_get_health` call
/// (id 9) plus whether any `forge_set_project` (id 8) was refused.
fn run_server(cwd: &Path, env: &[(&str, &str)], calls: &[&str]) -> (String, bool) {
    let mut cmd = Command::new(bin());
    cmd.arg("mcp").current_dir(cwd).env("NO_COLOR", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn forge mcp");

    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .unwrap();
    for c in calls {
        writeln!(stdin, "{c}").unwrap();
    }
    drop(stdin); // EOF → server exits

    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut summary = String::new();
    let mut set_project_refused = false;
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let id = v.get("id").and_then(|i| i.as_u64());
        let text = v
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        match id {
            Some(9) => {
                if let Ok(h) = serde_json::from_str::<serde_json::Value>(text) {
                    summary = h
                        .get("summary")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
            Some(8) => {
                // handle_set_project's refusal is an error CallToolResult (isError + text).
                let is_error = v
                    .get("result")
                    .and_then(|r| r.get("isError"))
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                if is_error || text.contains("explicitly bound") {
                    set_project_refused = true;
                }
            }
            _ => {}
        }
    }
    (summary, set_project_refused)
}

const GET_HEALTH: &str = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"forge_get_health","arguments":{}}}"#;

fn set_project(path: &Path) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"forge_set_project","arguments":{{"path":"{}"}}}}}}"#,
        path.display()
    )
}

#[test]
fn an_explicitly_bound_server_reads_its_own_project_despite_a_set_project_override() {
    // The contamination-as-a-test: consumer A is bound to projA; a second consumer's switch to
    // projB (the exact override the finding used) must be refused and A must still read projA.
    let proj_a = make_project("bound-a", true); // 100/100
    let proj_b = make_project("bound-b", false); // 90/100
    let elsewhere = std::env::temp_dir();

    let (summary, refused) = run_server(
        &elsewhere,
        &[("FORGE_PROJECT_ROOT", &proj_a.display().to_string())],
        &[&set_project(&proj_b), GET_HEALTH],
    );
    assert!(
        refused,
        "set_project must be refused under an explicit binding"
    );
    assert!(
        summary.contains("100/100"),
        "bound server must read its own project (projA=100), got: {summary}"
    );

    let _ = std::fs::remove_dir_all(&proj_a);
    let _ = std::fs::remove_dir_all(&proj_b);
}

#[test]
fn two_bound_consumers_each_read_their_own_project() {
    // Item 3: two concurrent consumers bound to different projects each read their own health.
    let proj_a = make_project("two-a", true); // 100
    let proj_b = make_project("two-b", false); // 90
    let elsewhere = std::env::temp_dir();

    let (a, _) = run_server(
        &elsewhere,
        &[("FORGE_PROJECT_ROOT", &proj_a.display().to_string())],
        &[GET_HEALTH],
    );
    let (b, _) = run_server(
        &elsewhere,
        &[("FORGE_PROJECT_ROOT", &proj_b.display().to_string())],
        &[GET_HEALTH],
    );
    assert!(a.contains("100/100"), "consumer A: {a}");
    assert!(b.contains("90/100"), "consumer B: {b}");
    assert_ne!(a, b, "two bound consumers must not read the same project");

    let _ = std::fs::remove_dir_all(&proj_a);
    let _ = std::fs::remove_dir_all(&proj_b);
}

#[test]
fn an_unbound_server_still_honours_set_project() {
    // Item 4 / DoD guard: the single-project switching UX must survive the fix. A server started
    // WITHOUT an explicit binding (cwd default, no env) may still switch via forge_set_project.
    let proj_a = make_project("unbound-a", true); // cwd = 100
    let proj_b = make_project("unbound-b", false); // switch target = 90

    let (summary, refused) = run_server(&proj_a, &[], &[&set_project(&proj_b), GET_HEALTH]);
    assert!(!refused, "an unbound server must accept set_project");
    assert!(
        summary.contains("90/100"),
        "unbound server must switch to projB (90), got: {summary}"
    );

    let _ = std::fs::remove_dir_all(&proj_a);
    let _ = std::fs::remove_dir_all(&proj_b);
}
