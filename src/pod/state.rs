//! Pod state store — `~/.cosmux/state.json`, read and written **in place**.
//!
//! Per the consolidation RFC's single-store protocol, forge uses the existing cosmux store at its
//! existing path. There is no import, no copy, and no new store path: a `forge pod` and a `cosmux`
//! invocation see the same file.
//!
//! # The test-isolation seam
//!
//! That store tracks **live fleet pods**. Writing it from a test would corrupt real pod tracking,
//! so `state_dir()` honours `FORGE_POD_STATE_DIR` — **defaulting to `~/.cosmux`, so production
//! behaviour is byte-identical to cosmux's**. The override exists for test isolation, not as an
//! alternative store.
//!
//! An override alone is not enough when the default is destructive: a test that *forgets* to set
//! it would silently write live state. So under `cfg(test)` every write is additionally asserted
//! to land under the temp directory — a forgetful test **panics** instead of touching the fleet.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::config::{PodConfig, expand_path};
use super::error::{PodError, Result};

/// Environment variable that redirects the pod store. Test isolation only — unset in production.
pub const STATE_DIR_ENV: &str = "FORGE_POD_STATE_DIR";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StateFile {
    #[serde(default)]
    pub pods: BTreeMap<String, PodState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodState {
    pub status: String,
    pub started_at: String,
    pub source_path: String,
    pub windows: Vec<WindowState>,
    #[serde(default)]
    pub on_pane_dead: Vec<String>,
    #[serde(default)]
    pub after_detach: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    pub name: String,
    pub panes: Vec<PaneState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaneState {
    pub index: usize,
    pub cwd: String,
    pub command: String,
    /// Optional `.forge` task binding. Absent on every pod cosmux wrote, which is why it is
    /// `skip_serializing_if` — forge must not rewrite the shared store with noise fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Resolve the store directory. `FORGE_POD_STATE_DIR` wins; otherwise the cosmux default.
pub fn state_dir() -> PathBuf {
    match std::env::var_os(STATE_DIR_ENV) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => expand_path("~/.cosmux"),
    }
}

pub fn state_path() -> PathBuf {
    state_dir().join("state.json")
}

/// Fail-closed guard: under test, a write outside the temp directory is a bug, not a mishap.
///
/// This is the difference between "the store *can* be redirected" and "a test that forgets to
/// redirect cannot damage the fleet".
#[cfg(test)]
fn assert_write_allowed(path: &Path) {
    let temp = std::env::temp_dir();
    assert!(
        path.starts_with(&temp),
        "REFUSING to write pod state outside the temp dir during tests: {}\n\
         The live store tracks running fleet pods. Set {STATE_DIR_ENV} to a temp path in this test.",
        path.display()
    );
}

#[cfg(not(test))]
fn assert_write_allowed(_path: &Path) {}

pub fn load() -> Result<StateFile> {
    load_from(&state_path())
}

/// Pure-ish read of a specific store file, so tests can exercise parsing without env juggling.
pub fn load_from(path: &Path) -> Result<StateFile> {
    if !path.exists() {
        return Ok(StateFile::default());
    }
    let raw = std::fs::read_to_string(path)?;
    parse(&raw)
}

/// Parse store contents. Separated from IO so the fixtures test real bytes, not a temp dir.
pub fn parse(raw: &str) -> Result<StateFile> {
    serde_json::from_str(raw).map_err(|e| PodError::Other(anyhow::anyhow!("state.json parse: {e}")))
}

/// Write the store.
///
/// This is the **single choke point** for production writes, so the adoption-journal authority
/// check lives here rather than on each verb: a future verb that writes the store cannot forget to
/// ask. Until `forge pod adopt` reaches terminal `adopted`, cosmux is the sole production writer
/// and this refuses (consolidation RFC §1 Phase A).
///
/// The check is a lock-free read of the journal — deliberately **not** the journal `flock`. Those
/// are two independent files with two independent mechanisms; coupling them would serialize
/// pod-store writes behind adoption bookkeeping for no safety gain.
pub fn save(state: &StateFile) -> Result<()> {
    super::journal::require_production_write()?;
    let dir = state_dir();
    let path = dir.join("state.json");
    assert_write_allowed(&path);
    std::fs::create_dir_all(&dir)?;
    let raw = serde_json::to_string_pretty(state)
        .map_err(|e| PodError::Other(anyhow::anyhow!("state.json serialize: {e}")))?;
    std::fs::write(&path, raw)?;
    Ok(())
}

/// Build the state entry for a pod without touching disk.
///
/// Split out of `record_spawn` so the shape can be asserted in tests with no store at all.
pub fn build_pod_state(pod: &PodConfig, source_path: &Path, started_at: String) -> PodState {
    let pod_root = pod.expanded_root();

    let windows = pod
        .windows
        .iter()
        .map(|w| WindowState {
            name: w.name.clone(),
            panes: w
                .panes
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let cwd = match (&p.cwd, &pod_root) {
                        (Some(c), _) => expand_path(c).display().to_string(),
                        (None, Some(r)) => r.display().to_string(),
                        (None, None) => String::from("."),
                    };
                    PaneState {
                        index: i,
                        cwd,
                        command: p.command.clone().unwrap_or_default(),
                        task: p.task.clone(),
                    }
                })
                .collect(),
        })
        .collect();

    PodState {
        status: "running".into(),
        started_at,
        source_path: source_path.display().to_string(),
        windows,
        on_pane_dead: pod.on_pane_dead.clone(),
        after_detach: pod.after_detach.clone(),
    }
}

pub fn record_spawn(pod: &PodConfig, source_path: &Path) -> Result<()> {
    let mut state = load()?;
    state.pods.insert(
        pod.name.clone(),
        build_pod_state(pod, source_path, now_iso8601()),
    );
    save(&state)
}

pub fn record_stop(name: &str) -> Result<()> {
    let mut state = load()?;
    state.pods.remove(name);
    save(&state)
}

pub fn pod(name: &str) -> Result<Option<PodState>> {
    Ok(load()?.pods.get(name).cloned())
}

/// cosmux hand-rolls civil-date arithmetic because it has no date dependency; forge already
/// depends on `chrono`, so the same ISO-8601 `Z` output comes from the library instead.
fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes every test that mutates a pod isolation env var.
    ///
    /// Env vars are process-global, so cargo's parallel test threads race on them: one test
    /// clearing the override while another is mid-write made the second resolve to the LIVE store.
    /// The fail-closed guard caught it — the run panicked instead of writing `~/.cosmux` — which
    /// is precisely the outcome it exists for, but the race itself is a test bug, fixed here.
    /// Shared with the journal tests, which redirect a different variable in the same process.
    use crate::pod::ENV_LOCK;

    /// Redirect the store **and** the adoption journal for the duration of a test.
    ///
    /// The journal comes along because `save` now refuses without terminal `adopted` authority: a
    /// store test is testing the store, not the authority gate, so the guard grants it explicitly
    /// (and visibly) rather than the gate having a test-only bypass.
    struct StoreGuard {
        _dir: tempdir::TempDir,
        _journal_dir: tempdir::TempDir,
        previous: Option<std::ffi::OsString>,
        previous_journal: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    /// Minimal temp-dir helper — no dev-dependency needed for one directory.
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new(tag: &str) -> Self {
                let base = std::env::temp_dir().join(format!(
                    "forge-pod-{}-{}-{:?}",
                    tag,
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::remove_dir_all(&base);
                std::fs::create_dir_all(&base).expect("create temp store");
                Self(base)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    impl StoreGuard {
        /// Redirect both seams and grant write authority via an adopted journal.
        fn new(tag: &str) -> Self {
            Self::build(tag, true)
        }

        /// Redirect both seams but leave the journal absent — i.e. **unadopted**, the Phase A
        /// production posture in which cosmux is the sole writer.
        fn unadopted(tag: &str) -> Self {
            Self::build(tag, false)
        }

        fn build(tag: &str, adopted: bool) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempdir::TempDir::new(tag);
            let journal_dir = tempdir::TempDir::new(&format!("{tag}-journal"));

            let previous = std::env::var_os(STATE_DIR_ENV);
            let previous_journal = std::env::var_os(super::super::journal::JOURNAL_DIR_ENV);
            unsafe { std::env::set_var(STATE_DIR_ENV, dir.path()) };
            unsafe {
                std::env::set_var(super::super::journal::JOURNAL_DIR_ENV, journal_dir.path())
            };

            if adopted {
                let mut journal = super::super::journal::Journal::new(
                    super::super::journal::TransitionState::Adopted,
                );
                journal.steps.shim = super::super::journal::StepState::Complete;
                std::fs::write(
                    journal_dir.path().join("pod-adoption.json"),
                    serde_json::to_string_pretty(&journal).unwrap(),
                )
                .unwrap();
            }

            Self {
                _dir: dir,
                _journal_dir: journal_dir,
                previous,
                previous_journal,
                _lock: lock,
            }
        }
    }

    impl Drop for StoreGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var(STATE_DIR_ENV, value) },
                None => unsafe { std::env::remove_var(STATE_DIR_ENV) },
            }
            match &self.previous_journal {
                Some(value) => unsafe {
                    std::env::set_var(super::super::journal::JOURNAL_DIR_ENV, value)
                },
                None => unsafe { std::env::remove_var(super::super::journal::JOURNAL_DIR_ENV) },
            }
        }
    }

    const LIVE_SHAPE: &str = r#"{
  "pods": {
    "Dx3_Program": {
      "status": "running",
      "started_at": "2026-04-19T21:59:48Z",
      "source_path": "/home/axw/ASIF/infra/tmux/Dx3_Program.yaml",
      "windows": [
        { "name": "claude", "panes": [ { "index": 0, "cwd": "/home/axw/projects/dx3", "command": "ccyolo" } ] }
      ]
    }
  }
}"#;

    #[test]
    fn parses_the_live_store_shape() {
        // Byte-shape taken from the real ~/.cosmux/state.json (read-only) so the vendored structs
        // are proven against what cosmux actually writes, not against an idealized fixture.
        let state = parse(LIVE_SHAPE).expect("live shape must parse");
        let pod = state.pods.get("Dx3_Program").expect("pod present");
        assert_eq!(pod.status, "running");
        assert_eq!(pod.windows[0].panes[0].command, "ccyolo");
        assert!(
            pod.on_pane_dead.is_empty(),
            "absent hook lists default to empty, not an error"
        );
        assert_eq!(pod.windows[0].panes[0].task, None);
    }

    #[test]
    fn round_trip_does_not_introduce_a_task_field() {
        // forge shares this store with cosmux. Serializing a cosmux-written pod must not inject
        // `task: null` noise into a file cosmux also reads.
        let state = parse(LIVE_SHAPE).unwrap();
        let out = serde_json::to_string(&state).unwrap();
        assert!(!out.contains("task"), "{out}");
    }

    #[test]
    fn state_dir_defaults_to_the_cosmux_store() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(STATE_DIR_ENV);
        unsafe { std::env::remove_var(STATE_DIR_ENV) };
        let resolved = state_dir();
        if let Some(value) = previous {
            unsafe { std::env::set_var(STATE_DIR_ENV, value) };
        }
        assert!(
            resolved.ends_with(".cosmux"),
            "production must use the cosmux store in place, got {}",
            resolved.display()
        );
    }

    #[test]
    fn save_and_load_round_trip_within_the_temp_store() {
        let _guard = StoreGuard::new("roundtrip");
        let mut state = StateFile::default();
        state.pods.insert(
            "test-pod".into(),
            PodState {
                status: "running".into(),
                started_at: "2026-07-18T00:00:00Z".into(),
                source_path: "/tmp/test-pod.yaml".into(),
                windows: vec![],
                on_pane_dead: vec![],
                after_detach: vec![],
            },
        );
        save(&state).expect("save to temp store");
        assert_eq!(load().expect("load back"), state);
        assert!(
            state_path().starts_with(std::env::temp_dir()),
            "guard must redirect the store"
        );
    }

    #[test]
    #[should_panic(expected = "REFUSING to write pod state outside the temp dir")]
    fn writing_outside_temp_panics_under_test() {
        // The fail-closed guard itself. This is the test that proves a forgetful test cannot
        // damage the live fleet store.
        assert_write_allowed(Path::new("/home/axw/.cosmux/state.json"));
    }

    #[test]
    fn a_store_write_is_refused_until_the_journal_says_adopted() {
        // Phase A: cosmux is the sole production writer. forge must refuse at the choke point, and
        // the refusal must name the remedy rather than failing opaquely.
        let _guard = StoreGuard::unadopted("refusal");
        let error = save(&StateFile::default()).expect_err("unadopted forge must not write");
        let message = error.to_string();
        assert!(message.contains("not adopted"), "{message}");
        assert!(message.contains("forge pod adopt"), "{message}");
        assert!(
            !state_path().exists(),
            "a refused write must not create the store"
        );
    }

    #[test]
    fn a_corrupt_journal_freezes_store_writes_rather_than_allowing_them() {
        // Negative control for the fail-open arm: if `verdict`'s default were "allow", this passes
        // a write over a journal nobody can read.
        let guard = StoreGuard::unadopted("frozen");
        std::fs::write(
            guard._journal_dir.path().join("pod-adoption.json"),
            "{ trunc",
        )
        .unwrap();
        let error = save(&StateFile::default()).expect_err("a corrupt journal must freeze writes");
        assert!(error.to_string().contains("recovery"), "{error}");
    }

    #[test]
    fn missing_store_loads_as_empty_not_an_error() {
        let _guard = StoreGuard::new("missing");
        let state = load().expect("absent store is not an error");
        assert!(state.pods.is_empty());
    }

    #[test]
    fn now_is_iso8601_zulu() {
        let now = now_iso8601();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.ends_with('Z'), "{now}");
        assert_eq!(&now[4..5], "-", "{now}");
        assert_eq!(&now[10..11], "T", "{now}");
    }
}
