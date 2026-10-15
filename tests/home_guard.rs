//! Tests must not write to the developer's real HOME, and must not start the real `agy`.
//! `forge sync` renders the agy adapter config, which adds the project to
//! `$HOME/.gemini/antigravity-cli/settings.json`; run with the real HOME, the suite added one
//! `/tmp/.tmpXXXX` entry per test project to the developer's own agy config (142 of them on
//! 2026-10-15). `common::forge()` / `common::forge_std()` give every spawned forge a test HOME,
//! a stub `agy` first on PATH and `BROWSER=/bin/false`. These tests fail if that stops holding.

use std::fs;
use std::path::{Path, PathBuf};

mod common;

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read test dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn tests_spawn_forge_only_through_the_shared_helper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("tests"), &mut files);
    // The helper itself, and this guard (whose code names the patterns).
    let allowed = [
        root.join("tests/common/mod.rs"),
        root.join("tests/home_guard.rs"),
    ];
    let patterns = [
        concat!("CARGO_BIN_EXE_", "forge"),
        concat!("cargo_bin(", "\"forge\")"),
    ];
    let mut offenders = Vec::new();
    for path in files.iter().filter(|p| !allowed.contains(p)) {
        let text = fs::read_to_string(path).expect("read test file");
        for (n, line) in text.lines().enumerate() {
            if patterns.iter().any(|p| line.contains(p)) {
                let rel = path.strip_prefix(root).unwrap().display();
                offenders.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    assert!(files.len() > allowed.len(), "found no test files to check");
    assert!(
        offenders.is_empty(),
        "spawn forge through common::forge() or common::forge_std() so it gets the test HOME:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_test_home_is_not_the_real_home() {
    let test_home = common::test_home();
    assert!(test_home.starts_with(env!("CARGO_TARGET_TMPDIR")));
    if let Some(real) = std::env::var_os("HOME") {
        assert_ne!(
            test_home,
            Path::new(&real),
            "the test HOME is the caller's HOME"
        );
    }
    let env = common::test_env();
    let get = |k: &str| {
        env.iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("HOME").as_deref(), Some(test_home.as_os_str()));
    assert_eq!(get("BROWSER").as_deref(), Some("/bin/false".as_ref()));
}

/// `forge init` then `forge sync` on a fresh project: the agy settings entry lands under the test
/// HOME, the real HOME's agy settings never name the project, and the agy that ran was the stub.
#[cfg(unix)]
#[test]
fn forge_sync_writes_agy_settings_only_under_the_test_home() {
    let dir = tempfile::TempDir::new().unwrap();
    let project = dir.path().to_str().unwrap().to_string();
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    common::forge()
        .args(["--project", &project, "init"])
        .assert()
        .success();
    common::forge()
        .args(["--project", &project, "sync"])
        .assert()
        .success();

    let rel = Path::new(".gemini/antigravity-cli/settings.json");
    let fake = fs::read_to_string(common::test_home().join(rel))
        .expect("forge sync wrote no agy settings under the test HOME");
    assert!(
        fake.contains(&name),
        "test HOME agy settings do not list the project"
    );

    if let Some(real_home) = std::env::var_os("HOME") {
        if let Ok(real) = fs::read_to_string(Path::new(&real_home).join(rel)) {
            assert!(
                !real.contains(&name),
                "a test wrote its project into the real HOME's agy settings"
            );
        }
    }

    let state = fs::read_to_string(dir.path().join(".forge/state.json")).unwrap();
    assert!(
        state.contains("0.0.0-forge-test-stub"),
        "forge detected an agy other than the test stub"
    );
}
