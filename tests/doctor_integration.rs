//! End-to-end `forge doctor` fixtures — one seeded defect class per test.
//!
//! DIRECTIVE-NXTG-20260718-07 DoD: `forge doctor --strict` must exit non-zero on each seeded
//! defect and zero on a clean repo. These drive the real binary, so they check the wiring
//! (exit code, JSON schema, flag handling) that the pure unit tests in `core::doctor` cannot.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Scratch directory that cleans itself up.
struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge-doctor-{}-{}-{}",
            name,
            std::process::id(),
            // Distinguish fixtures created within the same process.
            name.len()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create fixture dir");
        Self { path }
    }

    fn write(&self, rel: &str, contents: &str) -> &Self {
        let target = self.path.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(target, contents).expect("write fixture file");
        self
    }

    /// Seed a git repo with one commit and an optional tag, so release-debt has real git state.
    fn git_init(&self, tag: Option<&str>) -> &Self {
        run(&self.path, "git", &["init", "-q"]);
        run(
            &self.path,
            "git",
            &["config", "user.email", "t@example.com"],
        );
        run(&self.path, "git", &["config", "user.name", "t"]);
        run(&self.path, "git", &["add", "-A"]);
        run(
            &self.path,
            "git",
            &["commit", "-q", "--no-gpg-sign", "-m", "fixture"],
        );
        if let Some(tag) = tag {
            run(&self.path, "git", &["tag", tag]);
        }
        self
    }

    fn doctor(&self, args: &[&str]) -> (i32, String) {
        // Invoke the built binary directly. `cargo run` would resolve the FIXTURE's Cargo.toml
        // (each fixture is its own crate) and try to build that instead of forge.
        let mut full = vec!["doctor"];
        full.extend_from_slice(args);
        let output = Command::new(env!("CARGO_BIN_EXE_forge"))
            .args(&full)
            .current_dir(&self.path)
            .env("NO_COLOR", "1")
            .output()
            .expect("run forge doctor");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).to_string(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run(cwd: &Path, program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {program}: {e}"));
    assert!(
        status.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

const CLEAN_CARGO_TOML: &str = r#"[package]
name = "fixture-crate"
version = "1.0.0"
edition = "2021"
"#;

const CLEAN_CARGO_LOCK: &str = r#"version = 3

[[package]]
name = "fixture-crate"
version = "1.0.0"
"#;

#[test]
fn clean_repo_exits_zero_even_under_strict() {
    let fx = Fixture::new("clean");
    fx.write("Cargo.toml", CLEAN_CARGO_TOML)
        .write("Cargo.lock", CLEAN_CARGO_LOCK)
        .git_init(Some("v1.0.0"));

    let (code, out) = fx.doctor(&["--strict"]);
    assert_eq!(code, 0, "clean repo must pass --strict:\n{out}");
    assert!(out.contains("FORGE DOCTOR: OK"), "{out}");
}

#[test]
fn cargo_lock_desync_fails_closed_without_strict() {
    // The v1.5.1 incident: Cargo.toml bumped, Cargo.lock left behind (eaf8532, fixed in e319616).
    let fx = Fixture::new("lockdesync");
    fx.write(
        "Cargo.toml",
        r#"[package]
name = "fixture-crate"
version = "1.5.1"
"#,
    )
    .write(
        "Cargo.lock",
        r#"version = 3

[[package]]
name = "fixture-crate"
version = "1.5.0"
"#,
    )
    .git_init(Some("v1.5.0"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(
        code, 1,
        "lockfile desync is FAIL, which is fail-closed even without --strict:\n{out}"
    );
    assert!(out.contains("lockfile-desync"), "{out}");
}

#[test]
fn multi_surface_disagreement_fails() {
    // A single-manifest check reads plugin.json=2.0.0 and reports healthy; the disagreement with
    // package.json is only visible when every surface is compared (Codex finding 5).
    let fx = Fixture::new("multisurface");
    fx.write(".claude-plugin/plugin.json", r#"{"version":"2.0.0"}"#)
        .write("package.json", r#"{"name":"p","version":"1.0.0"}"#)
        .git_init(Some("v2.0.0"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 1, "surfaces disagree, must FAIL:\n{out}");
    assert!(out.contains("multi-surface-drift"), "{out}");
    assert!(out.contains("1.0.0") && out.contains("2.0.0"), "{out}");
}

#[test]
fn tag_drift_warns_and_only_strict_fails() {
    let fx = Fixture::new("tagdrift");
    fx.write(
        "Cargo.toml",
        r#"[package]
name = "fixture-crate"
version = "2.0.0"
"#,
    )
    .git_init(Some("v1.0.0"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 0, "WARN does not fail without --strict:\n{out}");
    assert!(out.contains("tag-drift"), "{out}");

    let (strict_code, strict_out) = fx.doctor(&["--strict"]);
    assert_eq!(
        strict_code, 1,
        "--strict escalates WARN to a non-zero exit:\n{strict_out}"
    );
}

#[test]
fn json_output_carries_the_schema_and_verdict() {
    let fx = Fixture::new("json");
    fx.write("Cargo.toml", CLEAN_CARGO_TOML)
        .write("Cargo.lock", CLEAN_CARGO_LOCK)
        .git_init(Some("v1.0.0"));

    let (code, out) = fx.doctor(&["--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(value["schema"], "forge.doctor.report.v1");
    assert_eq!(value["overall"], "OK");
    assert!(
        value["checks"].as_array().is_some_and(|c| c.len() == 3),
        "all three dimensions must appear: {out}"
    );
}

#[test]
fn uninitialized_project_skips_governance_rather_than_failing_it() {
    // A repo with no .forge/state.json is not a forge project. Reporting "Missing state.json"
    // as a critical health defect would be a category error — forge-orchestrator's own repo is
    // exactly this shape.
    let fx = Fixture::new("uninit");
    fx.write("Cargo.toml", CLEAN_CARGO_TOML)
        .write("Cargo.lock", CLEAN_CARGO_LOCK)
        .git_init(Some("v1.0.0"));

    let (code, out) = fx.doctor(&["--json", "--strict"]);
    assert_eq!(code, 0, "not-a-forge-project is not a failure:\n{out}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let quality = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "quality")
        .expect("quality dimension present");
    assert_eq!(quality["status"], "SKIP");
}
