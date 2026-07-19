//! Adoption transition journal — `~/.local/state/forge/pod-adoption.json`.
//!
//! This is the **only** representation of Forge production-write authority (consolidation RFC
//! §1.5, round-6 C2). It is a dedicated file, never a field inside the pod store: the canonical
//! store `~/.cosmux/state.json` stays byte-identical through every adoption, rollback, and repair,
//! which is exactly what lets the standalone rollback script `rm` the journal without being able to
//! damage pod state.
//!
//! # Two mechanisms, deliberately separate
//!
//! 1. **Journal read-modify-write is serialized by one `flock`** on the sibling
//!    `pod-adoption.lock`. Atomic replacement prevents a *torn file*; it does not serialize RMW, so
//!    two appends without a lock lose one another's writes (round-8 C4). Every mutation path —
//!    `_pane-recover` appends, drain updates, `--abort`, `--repair`, unadopt steps, and the terminal
//!    commit — goes through [`with_lock`].
//!
//!    The lock lives on its **own** file, never the journal, because [`JournalLock::write`] swaps
//!    the journal's inode via temp+rename: a lock held on the pre-rename inode would be silently
//!    orphaned and a second writer would walk straight in.
//!
//! 2. **Authority is a lock-free atomic read** ([`authority`]). That is safe precisely *because*
//!    journal writes are atomic renames, so a reader never sees a partial file. Pod-store writes
//!    must not take the journal lock — the two files stay independent, and what closes the
//!    transition window is that `adopt`/`unadopt` write their transition state **before** touching
//!    any other surface, so authority refuses from the first instant of a transition.
//!
//! # Fail-closed
//!
//! Absent, unreadable, wrong-schema, unknown-state, or `adopted`-with-pending-steps all resolve to
//! "no authority". The default arm of the verdict is refusal. This is the same silent-swallow class
//! that CRUCIBLE Gate 5 and the doctor D2 finding were both about: a gate that cannot read its own
//! input must not conclude "fine".

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::expand_path;
use super::error::{PodError, Result};

/// Redirects the journal + lock. Test isolation only — unset in production.
pub const JOURNAL_DIR_ENV: &str = "FORGE_POD_JOURNAL_DIR";

/// Schema version this binary understands. Anything else freezes into `recovery`.
pub const SCHEMA_VERSION: u32 = 1;

const JOURNAL_FILE: &str = "pod-adoption.json";
const LOCK_FILE: &str = "pod-adoption.lock";

pub fn journal_dir() -> PathBuf {
    match std::env::var_os(JOURNAL_DIR_ENV) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => expand_path("~/.local/state/forge"),
    }
}

pub fn journal_path() -> PathBuf {
    journal_dir().join(JOURNAL_FILE)
}

/// The lock file is a **sibling**, created once and never renamed or removed by a mutation.
pub fn lock_path() -> PathBuf {
    journal_dir().join(LOCK_FILE)
}

/// Where a transition currently stands. `unadopted` is represented by the journal's *absence*,
/// so it deliberately has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionState {
    Adopting,
    Adopted,
    Unadopting,
}

impl TransitionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Adopting => "adopting",
            Self::Adopted => "adopted",
            Self::Unadopting => "unadopting",
        }
    }
}

/// A ledger entry is written `pending` **before** its side effect and flipped to `complete` only
/// after that side effect is independently verified (§1.5 step-ledger semantics, round-5 C1b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Pending,
    Complete,
}

impl StepState {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steps {
    pub shim: StepState,
    /// One sub-entry per live tmux session: a multi-session rebind is N entries, not one.
    #[serde(default)]
    pub hooks: BTreeMap<String, StepState>,
    /// Durable respawn intents captured by `_pane-recover` during `adopting` (round-7 C3 — a
    /// process-local queue would strand recoveries across a restart).
    #[serde(default)]
    pub recoveries: BTreeMap<String, StepState>,
}

impl Default for Steps {
    fn default() -> Self {
        Self {
            shim: StepState::Pending,
            hooks: BTreeMap::new(),
            recoveries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    pub state: TransitionState,
    pub steps: Steps,
    pub ts: String,
    pub by: String,
}

impl Journal {
    pub fn new(state: TransitionState) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            state,
            steps: Steps::default(),
            ts: now_iso8601(),
            by: format!("forge@{}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Refresh the provenance fields on every mutation, so the journal always says who last wrote.
    pub fn touch(&mut self) {
        self.ts = now_iso8601();
        self.by = format!("forge@{}", env!("CARGO_PKG_VERSION"));
    }

    pub fn pending_recoveries(&self) -> Vec<&String> {
        self.steps
            .recoveries
            .iter()
            .filter(|(_, s)| !s.is_complete())
            .map(|(id, _)| id)
            .collect()
    }

    /// Every ledger entry complete — the precondition for a terminal state to be meaningful.
    pub fn all_steps_complete(&self) -> bool {
        self.steps.shim.is_complete()
            && self.steps.hooks.values().all(StepState::is_complete)
            && self.steps.recoveries.values().all(StepState::is_complete)
    }
}

/// The production-write authority verdict. Only [`Authority::Authorized`] permits a pod-store write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authority {
    /// Journal absent — cosmux is the sole writer (Phase A).
    Unadopted,
    /// A transition is in flight. Zero CLI writers by design; this is the brief write-freeze.
    InTransition(TransitionState),
    /// Terminal `adopted`, every step complete. The only state that authorizes a write.
    Authorized,
    /// Unreadable, partial, or semantically inconsistent → all production writes frozen until
    /// `forge pod adopt --repair` deterministically rolls back to `unadopted`.
    Recovery(String),
}

impl Authority {
    pub fn permits_production_write(&self) -> bool {
        matches!(self, Self::Authorized)
    }

    /// The message a refused verb prints. Each variant names the remedy, because the operator
    /// hitting this has no other signal about why a working command stopped working.
    pub fn refusal(&self) -> String {
        match self {
            Self::Authorized => String::new(),
            Self::Unadopted => "not adopted — run `forge pod adopt` (cosmux remains the sole \
                 production writer until then)"
                .into(),
            Self::InTransition(state) => format!(
                "adoption transition in progress (state: {}) — production writes are frozen until \
                 it completes; run `forge pod adopt --resume` or `--abort`",
                state.as_str()
            ),
            Self::Recovery(why) => format!(
                "adoption journal is in recovery: {why}\nAll forge production writes are frozen. \
                 Run `forge pod adopt --repair` (deterministic rollback to unadopted), then re-run \
                 `forge pod adopt` if adoption was the intent."
            ),
        }
    }
}

/// Decide authority from raw journal bytes — **pure**, so every arm is testable without a filesystem.
///
/// `None` means the file is absent. `Some(Err(_))` means it exists but could not be read: that is
/// *not* the same as absent, and it must never resolve to "unadopted".
pub fn verdict(raw: Option<std::result::Result<&str, String>>) -> Authority {
    let text = match raw {
        None => return Authority::Unadopted,
        Some(Err(error)) => return Authority::Recovery(format!("unreadable: {error}")),
        Some(Ok(text)) => text,
    };

    let journal: Journal = match serde_json::from_str(text) {
        Ok(journal) => journal,
        Err(error) => {
            // Covers truncation, corruption, an unknown `state` string, and a missing field.
            return Authority::Recovery(format!("malformed journal: {error}"));
        }
    };

    if journal.schema != SCHEMA_VERSION {
        return Authority::Recovery(format!(
            "schema {} is not the supported schema {SCHEMA_VERSION}",
            journal.schema
        ));
    }

    match journal.state {
        TransitionState::Adopting | TransitionState::Unadopting => {
            Authority::InTransition(journal.state)
        }
        TransitionState::Adopted if journal.all_steps_complete() => Authority::Authorized,
        // Semantically inconsistent: a terminal state can never coexist with a pending step.
        TransitionState::Adopted => Authority::Recovery(
            "state is `adopted` but the step ledger still has pending entries".into(),
        ),
    }
}

/// Read the journal and decide authority — lock-free, safe because writes are atomic renames.
pub fn authority() -> Authority {
    let path = journal_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => verdict(Some(Ok(&text))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Authority::Unadopted,
        Err(error) => verdict(Some(Err(error.to_string()))),
    }
}

/// The single choke point every pod-store write passes through.
pub fn require_production_write() -> Result<()> {
    let verdict = authority();
    if verdict.permits_production_write() {
        return Ok(());
    }
    Err(PodError::Other(anyhow::anyhow!(
        "refusing pod-store write: {}",
        verdict.refusal()
    )))
}

// ---------------------------------------------------------------------------------------------
// Locking
// ---------------------------------------------------------------------------------------------

/// Fail-closed guard: under test, a journal write outside the temp dir is a bug.
///
/// The live journal decides whether forge may write the fleet's pod store. A forgetful test that
/// wrote `adopted` there would silently hand a real machine an authority it never granted.
#[cfg(test)]
fn assert_write_allowed(path: &Path) {
    let temp = std::env::temp_dir();
    assert!(
        path.starts_with(&temp),
        "REFUSING to write the adoption journal outside the temp dir during tests: {}\n\
         Set {JOURNAL_DIR_ENV} to a temp path in this test.",
        path.display()
    );
}

#[cfg(not(test))]
fn assert_write_allowed(_path: &Path) {}

/// Exclusive access to the journal. Held for an entire read-modify-write, released on drop.
///
/// `flock(2)` here is the same primitive `flock(1)` uses, which is what lets the standalone
/// rollback script — which cannot depend on this binary — block against a live locked operation
/// (round-9 C5) rather than racing it.
pub struct JournalLock {
    file: File,
    dir: PathBuf,
    path: PathBuf,
}

impl JournalLock {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Raw journal bytes under the lock. `Ok(None)` means genuinely absent.
    pub fn read_raw(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Parsed journal under the lock. A present-but-malformed journal is an error, never `None` —
    /// callers must not be able to confuse "corrupt" with "absent".
    pub fn read(&self) -> Result<Option<Journal>> {
        match self.read_raw()? {
            None => Ok(None),
            Some(text) => serde_json::from_str(&text).map(Some).map_err(|e| {
                PodError::Other(anyhow::anyhow!("adoption journal is malformed: {e}"))
            }),
        }
    }

    /// Atomic replacement: write a sibling temp file, fsync, rename over the journal.
    pub fn write(&self, journal: &Journal) -> Result<()> {
        assert_write_allowed(&self.path);
        let raw = serde_json::to_string_pretty(journal)
            .map_err(|e| PodError::Other(anyhow::anyhow!("journal serialize: {e}")))?;
        let temp = self.dir.join(format!(
            ".pod-adoption.json.tmp.{}.{}",
            std::process::id(),
            now_nanos()
        ));
        {
            use std::io::Write as _;
            let mut handle = File::create(&temp)?;
            handle.write_all(raw.as_bytes())?;
            handle.write_all(b"\n")?;
            handle.sync_all()?;
        }
        std::fs::rename(&temp, &self.path)?;
        Ok(())
    }

    /// Remove the journal — the `unadopted` state. Never touches pod state (separate file, C2).
    pub fn remove(&self) -> Result<()> {
        assert_write_allowed(&self.path);
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for JournalLock {
    fn drop(&mut self) {
        // Closing the descriptor would release it anyway; being explicit documents the contract.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn open_lock_file() -> Result<(File, PathBuf, PathBuf)> {
    let dir = journal_dir();
    let lock = dir.join(LOCK_FILE);
    assert_write_allowed(&lock);
    std::fs::create_dir_all(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock)?;
    Ok((file, dir, journal_path()))
}

/// Run `f` holding the exclusive journal lock. Blocks until the lock is available.
pub fn with_lock<T>(f: impl FnOnce(&JournalLock) -> Result<T>) -> Result<T> {
    let (file, dir, path) = open_lock_file()?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let guard = JournalLock { file, dir, path };
    f(&guard)
}

/// Non-blocking variant. `Ok(None)` means another holder has it — used by probes and by the race
/// tests to assert exclusion without deadlocking the suite.
pub fn try_with_lock<T>(f: impl FnOnce(&JournalLock) -> Result<T>) -> Result<Option<T>> {
    let (file, dir, path) = open_lock_file()?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EINTR => Ok(None),
            _ => Err(error.into()),
        };
    }
    let guard = JournalLock { file, dir, path };
    f(&guard).map(Some)
}

// ---------------------------------------------------------------------------------------------
// Journal operations (all under the one lock)
// ---------------------------------------------------------------------------------------------

/// Outcome of appending a respawn intent during `adopting`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendOutcome {
    /// Recorded as `pending` — the drain will execute it.
    Appended,
    /// Already present; appends are idempotent so a re-fired hook cannot duplicate an entry.
    AlreadyPresent(StepState),
    /// No transition in flight — the caller should recover inline instead of deferring.
    NotAdopting,
}

/// Append a durable respawn intent for a pane. Read-modify-write **under the lock**, so two
/// concurrent hook firings serialize instead of overwriting one another.
pub fn append_recovery(pane_id: &str) -> Result<AppendOutcome> {
    with_lock(|lock| {
        let Some(mut journal) = lock.read()? else {
            return Ok(AppendOutcome::NotAdopting);
        };
        if journal.state != TransitionState::Adopting {
            return Ok(AppendOutcome::NotAdopting);
        }
        match journal.steps.recoveries.get(pane_id) {
            // Same occurrence still queued — dedup so a re-fired hook cannot duplicate it.
            Some(StepState::Pending) => {
                return Ok(AppendOutcome::AlreadyPresent(StepState::Pending));
            }
            // The pane already recovered once, then died AGAIN before the terminal commit — a NEW
            // occurrence (regate-15 P1-5). Re-pend it under this same lock so `pending_recoveries`
            // includes it, the drain re-runs it, and the terminal re-read-under-lock refuses to
            // commit while it is pending. Without this, a second death after the first completed
            // was stranded and the commit could authorize over it.
            Some(StepState::Complete) => {
                journal
                    .steps
                    .recoveries
                    .insert(pane_id.to_string(), StepState::Pending);
                journal.touch();
                lock.write(&journal)?;
                return Ok(AppendOutcome::Appended);
            }
            None => {}
        }
        journal
            .steps
            .recoveries
            .insert(pane_id.to_string(), StepState::Pending);
        journal.touch();
        lock.write(&journal)?;
        Ok(AppendOutcome::Appended)
    })
}

/// Flip a recovery entry to `complete` after its respawn is verified.
pub fn complete_recovery(pane_id: &str) -> Result<()> {
    with_lock(|lock| {
        let Some(mut journal) = lock.read()? else {
            return Ok(());
        };
        if journal.steps.recoveries.contains_key(pane_id) {
            journal
                .steps
                .recoveries
                .insert(pane_id.to_string(), StepState::Complete);
            journal.touch();
            lock.write(&journal)?;
        }
        Ok(())
    })
}

/// Result of attempting the terminal commit to `adopted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Journal is now terminal `adopted`; production writes are enabled.
    Committed,
    /// Recoveries appended after the drain snapshot are still pending — loop back to drain.
    /// This is the re-read-under-lock that stops an older snapshot from erasing a hook's append.
    PendingRecoveries(Vec<String>),
    /// A non-recovery ledger entry is incomplete; committing would be semantically inconsistent.
    IncompleteSteps,
    /// Nothing to commit.
    NotAdopting,
}

/// The FINAL operation of adoption: re-read under the lock, refuse `adopted` while any recovery is
/// pending, and only then replace the journal with the terminal state (§1.5 step 5).
pub fn commit_terminal() -> Result<CommitOutcome> {
    with_lock(|lock| {
        let Some(mut journal) = lock.read()? else {
            return Ok(CommitOutcome::NotAdopting);
        };
        if journal.state != TransitionState::Adopting {
            return Ok(CommitOutcome::NotAdopting);
        }
        let pending: Vec<String> = journal
            .pending_recoveries()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if !pending.is_empty() {
            return Ok(CommitOutcome::PendingRecoveries(pending));
        }
        if !journal.all_steps_complete() {
            return Ok(CommitOutcome::IncompleteSteps);
        }
        journal.state = TransitionState::Adopted;
        journal.touch();
        lock.write(&journal)?;
        Ok(CommitOutcome::Committed)
    })
}

fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env vars are process-global; cargo runs tests in parallel threads. Shared with the store
    /// tests so the two seams cannot race each other — see `pod::ENV_LOCK`.
    use crate::pod::ENV_LOCK;

    struct JournalGuard {
        dir: PathBuf,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl JournalGuard {
        fn new(tag: &str) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!(
                "forge-journal-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp journal dir");
            let previous = std::env::var_os(JOURNAL_DIR_ENV);
            unsafe { std::env::set_var(JOURNAL_DIR_ENV, &dir) };
            Self {
                dir,
                previous,
                _lock: lock,
            }
        }

        fn seed(&self, journal: &Journal) {
            let raw = serde_json::to_string_pretty(journal).unwrap();
            std::fs::write(self.dir.join(JOURNAL_FILE), raw).unwrap();
        }

        fn seed_raw(&self, raw: &str) {
            std::fs::write(self.dir.join(JOURNAL_FILE), raw).unwrap();
        }
    }

    impl Drop for JournalGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var(JOURNAL_DIR_ENV, value) },
                None => unsafe { std::env::remove_var(JOURNAL_DIR_ENV) },
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn adopted() -> Journal {
        let mut journal = Journal::new(TransitionState::Adopted);
        journal.steps.shim = StepState::Complete;
        journal
    }

    /// Mid-adoption with the shim step already verified — the state the hook-rebind and drain
    /// phases actually run in. A bare `Journal::new(Adopting)` has `shim: pending`, so a commit
    /// over it is refused for that reason and would never reach the recovery check under test.
    fn adopting_past_shim() -> Journal {
        let mut journal = Journal::new(TransitionState::Adopting);
        journal.steps.shim = StepState::Complete;
        journal
    }

    // -- authority verdict (pure) --------------------------------------------------------------

    #[test]
    fn absent_journal_is_unadopted_not_authorized() {
        assert_eq!(verdict(None), Authority::Unadopted);
        assert!(!verdict(None).permits_production_write());
    }

    #[test]
    fn unreadable_journal_freezes_rather_than_falling_back_to_unadopted() {
        // The fail-open trap: "cannot read it" must never be treated as "it isn't there".
        let v = verdict(Some(Err("permission denied".into())));
        assert!(matches!(v, Authority::Recovery(_)), "{v:?}");
        assert!(!v.permits_production_write());
    }

    #[test]
    fn truncated_and_corrupt_journals_freeze() {
        for raw in ["", "{", "{\"schema\":1,\"state\":\"ado", "null", "[]"] {
            let v = verdict(Some(Ok(raw)));
            assert!(matches!(v, Authority::Recovery(_)), "{raw:?} → {v:?}");
        }
    }

    #[test]
    fn unknown_state_string_freezes() {
        let raw = r#"{"schema":1,"state":"halfway","steps":{"shim":"complete"},"ts":"t","by":"x"}"#;
        assert!(matches!(verdict(Some(Ok(raw))), Authority::Recovery(_)));
    }

    #[test]
    fn wrong_schema_freezes() {
        let raw = r#"{"schema":2,"state":"adopted","steps":{"shim":"complete"},"ts":"t","by":"x"}"#;
        let v = verdict(Some(Ok(raw)));
        assert!(matches!(v, Authority::Recovery(_)), "{v:?}");
    }

    #[test]
    fn transitional_states_do_not_authorize_writes() {
        for state in [TransitionState::Adopting, TransitionState::Unadopting] {
            let raw = serde_json::to_string(&Journal::new(state)).unwrap();
            let v = verdict(Some(Ok(&raw)));
            assert_eq!(v, Authority::InTransition(state));
            assert!(!v.permits_production_write());
        }
    }

    #[test]
    fn adopted_with_a_pending_step_is_inconsistent_and_freezes() {
        // A terminal state can never coexist with a pending entry; treating it as authoritative
        // would authorize writes over a half-rebound surface.
        let mut journal = adopted();
        journal
            .steps
            .hooks
            .insert("sess".into(), StepState::Pending);
        let raw = serde_json::to_string(&journal).unwrap();
        assert!(matches!(verdict(Some(Ok(&raw))), Authority::Recovery(_)));

        let mut journal = adopted();
        journal
            .steps
            .recoveries
            .insert("%1".into(), StepState::Pending);
        let raw = serde_json::to_string(&journal).unwrap();
        assert!(matches!(verdict(Some(Ok(&raw))), Authority::Recovery(_)));
    }

    #[test]
    fn only_terminal_adopted_with_every_step_complete_authorizes() {
        let mut journal = adopted();
        journal
            .steps
            .hooks
            .insert("sess".into(), StepState::Complete);
        journal
            .steps
            .recoveries
            .insert("%1".into(), StepState::Complete);
        let raw = serde_json::to_string(&journal).unwrap();
        assert_eq!(verdict(Some(Ok(&raw))), Authority::Authorized);
        assert!(verdict(Some(Ok(&raw))).permits_production_write());
    }

    #[test]
    fn every_refusal_names_a_remedy() {
        // The operator hitting a refusal has no other signal about why a working verb stopped.
        for v in [
            Authority::Unadopted,
            Authority::InTransition(TransitionState::Adopting),
            Authority::Recovery("truncated".into()),
        ] {
            let message = v.refusal();
            assert!(!message.is_empty(), "{v:?}");
            assert!(message.contains("forge pod"), "{v:?} → {message}");
        }
    }

    // -- authority against the filesystem ------------------------------------------------------

    #[test]
    fn authority_reads_the_journal_at_the_configured_path() {
        let guard = JournalGuard::new("authority");
        assert_eq!(authority(), Authority::Unadopted);
        assert!(require_production_write().is_err());

        guard.seed(&adopted());
        assert_eq!(authority(), Authority::Authorized);
        assert!(require_production_write().is_ok());

        guard.seed_raw("{ truncated");
        assert!(matches!(authority(), Authority::Recovery(_)));
        assert!(
            require_production_write().is_err(),
            "a corrupt journal must not authorize a pod-store write"
        );
    }

    // -- locking -------------------------------------------------------------------------------

    #[test]
    fn the_lock_is_a_sibling_file_never_the_journal() {
        // temp+rename swaps the journal's inode. A lock held on the journal itself would be
        // orphaned by the very write it is supposed to be guarding.
        let _guard = JournalGuard::new("lockpath");
        assert_ne!(lock_path(), journal_path());
        assert_eq!(lock_path().file_name().unwrap(), LOCK_FILE);
    }

    #[test]
    fn a_second_acquisition_is_excluded_while_the_first_is_held() {
        let _guard = JournalGuard::new("exclude");
        with_lock(|_held| {
            // Separate open file descriptions, so flock excludes even within one process.
            let attempted = try_with_lock(|_| Ok(()))?;
            assert_eq!(attempted, None, "the lock must exclude a second holder");
            Ok(())
        })
        .unwrap();

        // …and is released on drop.
        assert_eq!(try_with_lock(|_| Ok(())).unwrap(), Some(()));
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temp_files_behind() {
        let guard = JournalGuard::new("atomic");
        with_lock(|lock| lock.write(&Journal::new(TransitionState::Adopting))).unwrap();

        let strays: Vec<_> = std::fs::read_dir(&guard.dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(strays.is_empty(), "temp files left behind: {strays:?}");
        assert_eq!(
            authority(),
            Authority::InTransition(TransitionState::Adopting)
        );
    }

    #[test]
    fn read_distinguishes_absent_from_malformed() {
        let guard = JournalGuard::new("absent");
        with_lock(|lock| {
            assert!(lock.read()?.is_none(), "absent journal reads as None");
            Ok(())
        })
        .unwrap();

        guard.seed_raw("{ nope");
        let result = with_lock(|lock| lock.read());
        assert!(
            result.is_err(),
            "a malformed journal must be an error, never None — None means unadopted"
        );
    }

    #[test]
    fn remove_leaves_the_lock_file_intact() {
        // The standalone script removes the journal while holding this same lock; if `remove` also
        // took out the lock file, the lock it holds would be on an unlinked inode.
        let guard = JournalGuard::new("remove");
        guard.seed(&adopted());
        with_lock(|lock| lock.remove()).unwrap();
        assert!(!journal_path().exists());
        assert!(lock_path().exists(), "lock file must survive a journal rm");
        assert_eq!(authority(), Authority::Unadopted);
    }

    // -- cross-primitive interop with flock(1) -------------------------------------------------

    fn flock_binary_available() -> bool {
        std::process::Command::new("flock")
            .arg("--help")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn the_shell_flock_and_the_binary_exclude_each_other() {
        // The standalone rollback script cannot depend on the forge binary, so it locks with
        // `flock(1)`. If that did not exclude this binary's `flock(2)`, the "rollback blocks until
        // release" acceptance leg would pass while the two actually raced — every other race test
        // in this module would be proving the wrong thing.
        if !flock_binary_available() {
            eprintln!("SKIP: flock(1) not on PATH — cross-primitive interop unverified");
            return;
        }

        let guard = JournalGuard::new("interop");
        let lock = guard.dir.join(LOCK_FILE);
        std::fs::write(&lock, b"").unwrap();
        let ready = guard.dir.join("holding");

        // Direction 1: the shell holds it, this binary must be excluded.
        let mut child = std::process::Command::new("flock")
            .arg("-x")
            .arg(&lock)
            .arg("-c")
            // Single-quoted: the temp dir name embeds a `ThreadId(n)`, and unquoted parentheses
            // are a `sh` syntax error — the child would exit before ever taking the lock.
            .arg(format!("touch '{}' && sleep 3", ready.display()))
            .spawn()
            .expect("spawn flock(1)");

        let mut waited = 0;
        while !ready.exists() && waited < 3000 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        assert!(ready.exists(), "flock(1) never took the lock");
        assert_eq!(
            try_with_lock(|_| Ok(())).unwrap(),
            None,
            "the binary must block on a lock held by flock(1)"
        );

        let _ = child.kill();
        let _ = child.wait();

        // Direction 2: this binary holds it, the shell must be excluded.
        let blocked = with_lock(|_held| {
            let status = std::process::Command::new("flock")
                .args(["-w", "0", "-x"])
                .arg(&lock)
                .args(["-c", "true"])
                .status()
                .expect("run flock(1)");
            Ok(status.success())
        })
        .unwrap();
        assert!(
            !blocked,
            "flock(1) must block on a lock held by the binary — the standalone rollback depends on it"
        );

        // …and succeeds once released, so the exclusion is the lock, not a broken invocation.
        let after = std::process::Command::new("flock")
            .args(["-w", "0", "-x"])
            .arg(&lock)
            .args(["-c", "true"])
            .status()
            .expect("run flock(1)");
        assert!(
            after.success(),
            "flock(1) must acquire once the binary releases"
        );
    }

    // -- recovery ledger races -----------------------------------------------------------------

    #[test]
    fn concurrent_recovery_appends_both_survive() {
        // The round-8 C4 leg: atomic replacement alone would let the second writer's snapshot
        // erase the first append. Serialized RMW is what keeps both.
        let guard = JournalGuard::new("append-race");
        guard.seed(&Journal::new(TransitionState::Adopting));

        std::thread::scope(|scope| {
            let handles: Vec<_> = ["%1", "%2", "%3", "%4"]
                .into_iter()
                .map(|pane| scope.spawn(move || append_recovery(pane).unwrap()))
                .collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), AppendOutcome::Appended);
            }
        });

        let journal = with_lock(|lock| lock.read()).unwrap().unwrap();
        assert_eq!(
            journal.steps.recoveries.len(),
            4,
            "every pane identifier must remain present: {:?}",
            journal.steps.recoveries
        );
    }

    #[test]
    fn appending_the_same_pane_twice_is_idempotent() {
        let guard = JournalGuard::new("append-idem");
        guard.seed(&Journal::new(TransitionState::Adopting));
        assert_eq!(append_recovery("%1").unwrap(), AppendOutcome::Appended);
        assert_eq!(
            append_recovery("%1").unwrap(),
            AppendOutcome::AlreadyPresent(StepState::Pending),
            "a re-fired hook must not duplicate an entry"
        );
        let journal = with_lock(|lock| lock.read()).unwrap().unwrap();
        assert_eq!(journal.steps.recoveries.len(), 1);
    }

    #[test]
    fn a_second_death_after_recovery_completes_re_pends_and_blocks_the_commit() {
        // regate-15 P1-5: the exact boundary. A pane recovers (Appended → complete), then dies
        // AGAIN before the terminal commit. That re-death must re-pend (a new occurrence), so the
        // drain re-runs it and the commit refuses — never strand it and authorize over it.
        let guard = JournalGuard::new("repend");
        guard.seed(&adopting_past_shim());

        assert_eq!(append_recovery("%1").unwrap(), AppendOutcome::Appended);
        complete_recovery("%1").unwrap();
        // Ledger now says %1 is complete; a naive commit would proceed.
        assert_eq!(commit_terminal().unwrap(), CommitOutcome::Committed);
        // (In the real flow the drain→commit happens while still adopting; re-seed that state to
        // exercise the re-death boundary deterministically.)
        guard.seed(&{
            let mut j = adopting_past_shim();
            j.steps.recoveries.insert("%1".into(), StepState::Complete);
            j
        });

        // Second death of the SAME pane, still adopting → must re-pend, not report AlreadyPresent.
        assert_eq!(
            append_recovery("%1").unwrap(),
            AppendOutcome::Appended,
            "a re-death after completion must be a fresh pending occurrence"
        );
        let journal = with_lock(|lock| lock.read()).unwrap().unwrap();
        assert_eq!(
            journal.steps.recoveries.get("%1"),
            Some(&StepState::Pending)
        );
        assert_eq!(journal.pending_recoveries(), vec![&"%1".to_string()]);

        // …and the terminal commit now refuses while it is pending.
        match commit_terminal().unwrap() {
            CommitOutcome::PendingRecoveries(p) => assert_eq!(p, vec!["%1"]),
            other => panic!("commit must wait for the re-pended pane, got {other:?}"),
        }
    }

    #[test]
    fn appends_outside_adopting_are_refused_not_silently_dropped() {
        let guard = JournalGuard::new("append-terminal");
        guard.seed(&adopted());
        assert_eq!(append_recovery("%1").unwrap(), AppendOutcome::NotAdopting);
        let journal = with_lock(|lock| lock.read()).unwrap().unwrap();
        assert!(journal.steps.recoveries.is_empty());
    }

    #[test]
    fn terminal_commit_refuses_while_a_recovery_is_pending() {
        // The re-read-under-lock. Without it, a hook append landing after the drain's snapshot is
        // erased by the terminal write and its pane is stranded, unrecovered, forever.
        let guard = JournalGuard::new("commit-pending");
        guard.seed(&adopting_past_shim());
        append_recovery("%late").unwrap();

        match commit_terminal().unwrap() {
            CommitOutcome::PendingRecoveries(pending) => assert_eq!(pending, vec!["%late"]),
            other => panic!("terminal commit must loop back to drain, got {other:?}"),
        }
        assert_eq!(
            authority(),
            Authority::InTransition(TransitionState::Adopting),
            "state must remain adopting — no authority is granted over a pending recovery"
        );

        complete_recovery("%late").unwrap();
        assert_eq!(commit_terminal().unwrap(), CommitOutcome::Committed);
        assert_eq!(authority(), Authority::Authorized);
    }

    #[test]
    fn an_append_racing_the_terminal_commit_is_never_lost() {
        // The append-vs-terminal leg: whichever order they serialize in, the pane must still be
        // present afterwards and must be accounted for exactly once.
        for attempt in 0..25 {
            let guard = JournalGuard::new(&format!("commit-race-{attempt}"));
            guard.seed(&adopting_past_shim());

            let (append, commit) = std::thread::scope(|scope| {
                let appender = scope.spawn(|| append_recovery("%racer").unwrap());
                let committer = scope.spawn(|| commit_terminal().unwrap());
                (appender.join().unwrap(), committer.join().unwrap())
            });

            let journal = with_lock(|lock| lock.read()).unwrap().unwrap();
            let recorded = journal.steps.recoveries.get("%racer");

            // Exactly one of the two disposals happens — never both, never neither. That is what
            // "completes exactly once" means at this boundary: the pane is either deferred to the
            // drain (still `adopting`) or handled inline by an authorized forge (`adopted`).
            match (&append, &commit) {
                // The append won: the commit re-read under the lock, saw it, and looped back to
                // drain instead of overwriting it with its older snapshot.
                (AppendOutcome::Appended, CommitOutcome::PendingRecoveries(pending)) => {
                    assert_eq!(pending, &vec!["%racer".to_string()], "attempt {attempt}");
                    assert_eq!(recorded, Some(&StepState::Pending), "attempt {attempt}");
                    assert_eq!(
                        journal.state,
                        TransitionState::Adopting,
                        "attempt {attempt}"
                    );
                }
                // The commit won: the journal is terminal, so the hook is refused a *deferral* —
                // correctly, because forge now holds authority and recovers the pane inline. The
                // intent is not lost, it is redirected to the live path.
                (AppendOutcome::NotAdopting, CommitOutcome::Committed) => {
                    assert_eq!(journal.state, TransitionState::Adopted, "attempt {attempt}");
                    assert_eq!(
                        recorded, None,
                        "attempt {attempt}: a terminal journal must not accrue deferred intents"
                    );
                    assert!(authority().permits_production_write(), "attempt {attempt}");
                }
                // Appended AND committed would mean the terminal write erased a live intent —
                // exactly the round-8 C4 defect the re-read-under-lock exists to prevent.
                other => panic!("attempt {attempt}: illegal interleaving {other:?}"),
            }
        }
    }

    #[test]
    fn commit_refuses_an_incomplete_non_recovery_step() {
        let guard = JournalGuard::new("commit-incomplete");
        let mut journal = Journal::new(TransitionState::Adopting);
        journal.steps.shim = StepState::Pending;
        guard.seed(&journal);
        assert_eq!(commit_terminal().unwrap(), CommitOutcome::IncompleteSteps);
        assert!(!authority().permits_production_write());
    }

    #[test]
    #[should_panic(expected = "REFUSING to write the adoption journal outside the temp dir")]
    fn writing_the_live_journal_panics_under_test() {
        assert_write_allowed(Path::new("/home/axw/.local/state/forge/pod-adoption.json"));
    }
}
