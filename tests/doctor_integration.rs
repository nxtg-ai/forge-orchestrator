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

/// Build a fixture mirroring the REAL forge-plugin layout: a marketplace manifest whose versions
/// live in `plugins[]`, a root plugin manifest, and a nested Node package with its own lockfile
/// three directories down.
fn plugin_shaped_fixture(fx: &Fixture, marketplace: &str, nested_pkg: &str, nested_lock: &str) {
    fx.write(
        ".claude-plugin/marketplace.json",
        &format!(
            r#"{{
  "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
  "name": "nxtg-forge",
  "plugins": [
    {{ "name": "nxtg-forge", "source": "./plugins/nxtg-forge", "version": "{marketplace}" }}
  ]
}}"#
        ),
    )
    .write(
        ".claude-plugin/plugin.json",
        &format!(r#"{{"name":"nxtg-forge","version":"{marketplace}"}}"#),
    )
    .write(
        "plugins/nxtg-forge/.claude-plugin/plugin.json",
        &format!(r#"{{"name":"nxtg-forge","version":"{marketplace}"}}"#),
    )
    .write(
        "plugins/nxtg-forge/servers/governance-mcp/package.json",
        &format!(r#"{{"name":"@nxtg-forge/governance-mcp","version":"{nested_pkg}"}}"#),
    )
    .write(
        "plugins/nxtg-forge/servers/governance-mcp/package-lock.json",
        &format!(r#"{{"name":"@nxtg-forge/governance-mcp","version":"{nested_lock}","lockfileVersion":3}}"#),
    );
}

#[test]
fn clean_plugin_layout_passes_marketplace_is_not_a_top_level_manifest() {
    // Codex round 4, direction 1: a CLEAN checkout reported FAIL because marketplace.json has no
    // top-level "version" — its versions live in plugins[]. Reading it generically produced
    // "version-unreadable" on a repo where every surface actually agreed.
    let fx = Fixture::new("pluginclean");
    plugin_shaped_fixture(&fx, "3.10.3", "3.10.3", "3.10.3");
    fx.git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&["--strict"]);
    assert_eq!(code, 0, "a clean plugin checkout must pass:\n{out}");
    assert!(!out.contains("version-unreadable"), "{out}");
}

#[test]
fn nested_manifest_drift_is_detected() {
    // Codex round 4, direction 2: a root-only scan never saw
    // servers/governance-mcp/package.json, so genuine drift three levels down reported PASS.
    let fx = Fixture::new("pluginnested");
    plugin_shaped_fixture(&fx, "3.10.3", "3.7.0", "3.7.0");
    fx.git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 1, "nested drift must FAIL:\n{out}");
    assert!(out.contains("multi-surface-drift"), "{out}");
    assert!(
        out.contains("governance-mcp/package.json"),
        "the finding must name the nested file: {out}"
    );
}

#[test]
fn nested_lockfile_desync_is_detected() {
    // The nested equivalent of the v1.5.1 Cargo.lock incident: manifest and its own lockfile
    // disagree, three directories below the repo root.
    let fx = Fixture::new("pluginlock");
    plugin_shaped_fixture(&fx, "3.10.3", "3.10.3", "3.10.2");
    fx.git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 1, "nested lockfile desync must FAIL:\n{out}");
    assert!(out.contains("lockfile-desync"), "{out}");
    assert!(out.contains("3.10.2"), "{out}");
}

#[test]
fn dependency_directories_are_not_inventoried() {
    // node_modules holds thousands of OTHER projects' manifests. Including them would compare
    // this repo's version against every transitive dependency and fail every real repo.
    let fx = Fixture::new("pluginexclude");
    plugin_shaped_fixture(&fx, "3.10.3", "3.10.3", "3.10.3");
    fx.write(
        "plugins/nxtg-forge/servers/governance-mcp/node_modules/left-pad/package.json",
        r#"{"name":"left-pad","version":"1.3.0"}"#,
    )
    .write(
        "target/debug/build/something/package.json",
        r#"{"version":"0.0.1"}"#,
    )
    .git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&["--strict"]);
    assert_eq!(
        code, 0,
        "dependency manifests must not be treated as this repo's surfaces:\n{out}"
    );
    assert!(!out.contains("1.3.0"), "{out}");
}

#[test]
fn marketplace_entry_missing_a_version_fails_strict() {
    // Codex round 5's exact reproduction: remove the version from a required plugin entry and the
    // strict gate exited 0, because filter_map dropped the entry before it could be judged.
    let fx = Fixture::new("mktmissing");
    fx.write(
        ".claude-plugin/marketplace.json",
        r#"{
  "name": "nxtg-forge",
  "plugins": [
    { "name": "nxtg-forge", "source": "./plugins/nxtg-forge", "version": "3.10.3" },
    { "name": "second-plugin", "source": "./plugins/second" }
  ]
}"#,
    )
    .write(
        ".claude-plugin/plugin.json",
        r#"{"name":"nxtg-forge","version":"3.10.3"}"#,
    )
    .git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&["--strict"]);
    assert_eq!(code, 1, "a versionless plugin entry must FAIL:\n{out}");
    assert!(out.contains("version-unreadable"), "{out}");
    assert!(
        out.contains("second-plugin"),
        "the finding must name the offending entry: {out}"
    );
}

#[test]
fn malformed_marketplace_structure_fails_rather_than_yielding_no_surfaces() {
    // `plugins` present but not an array previously produced an empty surface list, which is
    // indistinguishable from "this repo declares no plugins" — a broken file passing as clean.
    let fx = Fixture::new("mktmalformed");
    fx.write(
        ".claude-plugin/marketplace.json",
        r#"{"name":"nxtg-forge","plugins":{"oops":"not an array"}}"#,
    )
    .write(
        ".claude-plugin/plugin.json",
        r#"{"name":"nxtg-forge","version":"3.10.3"}"#,
    )
    .git_init(Some("v3.10.3"));

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 1, "a malformed manifest must FAIL:\n{out}");
    assert!(out.contains("inventory-error"), "{out}");
    assert!(out.contains("not an array"), "{out}");
}

#[test]
fn unreadable_surface_is_reported_not_skipped() {
    // A file the gate cannot read has not been verified. Skipping it silently lets a broken
    // checkout report clean.
    let fx = Fixture::new("unreadable");
    fx.write(
        ".claude-plugin/plugin.json",
        r#"{"name":"nxtg-forge","version":"3.10.3"}"#,
    )
    .write("package.json", r#"{"name":"x","version":"3.10.3"}"#)
    .git_init(Some("v3.10.3"));

    // Make one surface unreadable. Skipped when running as root, where mode bits do not apply.
    let target = fx.path.join("package.json");
    let mut perms = fs::metadata(&target).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o000);
    }
    fs::set_permissions(&target, perms).unwrap();
    if fs::read_to_string(&target).is_ok() {
        eprintln!("skipping: running with privileges that bypass file permissions");
        return;
    }

    let (code, out) = fx.doctor(&[]);
    assert_eq!(code, 1, "an unreadable surface must FAIL:\n{out}");
    assert!(out.contains("inventory-error"), "{out}");
    assert!(out.contains("package.json"), "{out}");
}
