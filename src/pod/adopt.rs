//! Adoption state machine — consolidation RFC §1.5.
//!
//! This drives the migration from cosmux to forge as the pod-store's production writer. It is built
//! **on top of** [`super::journal`], which is the sole authority signal: nothing here grants a write
//! by itself; every path funnels authority through the journal's terminal `adopted` state.
//!
//! # Deterministic adopt order (§1.5)
//!
//! 1. **Preflight** — refuse unless zero cosmux processes and zero mid-respawn panes; then write
//!    `adopting` **before any surface is touched**, so authority refuses from the first instant.
//! 2. **Shim** — install a `cosmux` → `forge pod` shim, so any lingering `cosmux` CLI invocation
//!    reaches forge (which refuses production writes until terminal). From here there are zero CLI
//!    writers: a brief write-freeze, never a dual-writer.
//! 3. **Hook rebind** — enumerate every live session, rewrite cosmux-exe hooks → forge, verify the
//!    readback per session. A pane-died hook firing mid-rebind that reaches forge's `_pane-recover`
//!    appends a durable recovery intent rather than acting (it has no authority yet).
//! 4. **Recovery drain** — execute every pending recovery, flip to `complete` after the respawn.
//! 5. **Terminal commit** — the FINAL write; production authority enables only now.
//!
//! # Rollback
//!
//! `unadopt` (and `--abort`, `--repair`) mirror the machine in reverse: restore every hook from the
//! backup captured at rebind, remove the shim, remove the journal. All three are also reachable from
//! the **standalone shell script**, which takes the same `flock` and so blocks against a live locked
//! operation instead of racing it — the script is why rollback never depends on the forge binary.
//!
//! # Test isolation
//!
//! Every destructive surface has an env seam that defaults to its live location, so production is
//! unchanged, and a `cfg(test)` guard that **panics** on a write outside the temp dir: the shim path
//! ([`SHIM_PATH_ENV`]), the tmux socket (`FORGE_POD_TMUX_SOCKET`), and the journal
//! (`FORGE_POD_JOURNAL_DIR`). Failure injection ([`FAIL_AT_ENV`]) is reachable **only** when the
//! journal dir is redirected, so it can never fire against the live journal.

use std::path::{Path, PathBuf};

use super::config::expand_path;
use super::error::{PodError, Result};
use super::journal::{self, Authority, CommitOutcome, StepState, TransitionState};
use super::tmux::{SessionHook, Tmux, current_exe, forge_hook};

/// Redirects the installed shim path. Test isolation only — unset in production.
pub const SHIM_PATH_ENV: &str = "FORGE_POD_SHIM_PATH";

/// Injects a simulated crash at a named point relative to a durable journal write. Reachable only
/// when `FORGE_POD_JOURNAL_DIR` is also set, so it is unreachable against the live journal.
pub const FAIL_AT_ENV: &str = "FORGE_POD_ADOPT_FAIL_AT";

/// Sentinel written in the hook-backup file when the original hook was unset — restore = `-u`.
const UNSET_SENTINEL: &str = "\u{0}UNSET";

// ---------------------------------------------------------------------------------------------
// Hook ownership — pure classification
// ---------------------------------------------------------------------------------------------

/// Who a live session hook currently points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOwner {
    /// No hook set.
    Absent,
    /// cosmux — invokes `<exe> _pane-recover` (no `pod` infix).
    Cosmux,
    /// forge — invokes `<exe> pod _pane-recover`.
    Forge,
    /// Something else entirely. Never touched: forge only rebinds cosmux's own hooks.
    Foreign,
}

/// Classify a hook value — **pure**. The discriminator is the `pod ` infix that forge's hook has
/// and cosmux's does not, matching [`forge_hook`] exactly. Both binaries invoke the same recovery
/// subcommands, so the presence of `_pane-recover`/`_after-detach` marks it as one of the two.
pub fn classify_hook(value: Option<&str>) -> HookOwner {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return HookOwner::Absent;
    };
    let is_recovery = value.contains("_pane-recover") || value.contains("_after-detach");
    if !is_recovery {
        return HookOwner::Foreign;
    }
    if value.contains(" pod _pane-recover") || value.contains(" pod _after-detach") {
        HookOwner::Forge
    } else {
        HookOwner::Cosmux
    }
}

// ---------------------------------------------------------------------------------------------
// Preflight — pure decision
// ---------------------------------------------------------------------------------------------

/// The adopt gate: refuse while a second writer could still act (§1.5 step 1). Pure so both refusal
/// reasons are tested without spawning processes.
pub fn adopt_preflight(cosmux_running: bool, mid_respawn_panes: bool) -> Result<()> {
    if cosmux_running {
        return Err(PodError::Other(anyhow::anyhow!(
            "refusing to adopt while a `cosmux` process is running — it could write the store \
             concurrently. Stop cosmux, then re-run `forge pod adopt`."
        )));
    }
    if mid_respawn_panes {
        return Err(PodError::Other(anyhow::anyhow!(
            "refusing to adopt while a pane is mid-respawn — its recovery could write the store. \
             Let it settle, then re-run `forge pod adopt`."
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Failure injection — reachable only against a redirected journal
// ---------------------------------------------------------------------------------------------

fn injection_enabled() -> bool {
    std::env::var_os(journal::JOURNAL_DIR_ENV).is_some_and(|v| !v.is_empty())
}

/// Simulate a crash at `marker` if `FORGE_POD_ADOPT_FAIL_AT` names it. Modelled as an error rather
/// than a real `exit`, so the harness can drive resume/repair; only durable disk state matters, and
/// the marker fires at an exact point relative to a journal write, exactly as a crash there would.
fn crash_if(marker: &str) -> Result<()> {
    if !injection_enabled() {
        return Ok(());
    }
    if std::env::var(FAIL_AT_ENV).ok().as_deref() == Some(marker) {
        return Err(PodError::Other(anyhow::anyhow!(
            "injected crash at {marker}"
        )));
    }
    Ok(())
}

/// True if the caller injected this exact crash — lets the orchestrator distinguish a modelled
/// crash (leave disk as-is, stop) from a genuine error (surface it).
fn is_injected(error: &PodError) -> bool {
    error.to_string().contains("injected crash at")
}

// ---------------------------------------------------------------------------------------------
// Shim — the `cosmux` → `forge pod` redirect
// ---------------------------------------------------------------------------------------------

pub fn shim_path() -> PathBuf {
    match std::env::var_os(SHIM_PATH_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => expand_path("~/.local/bin/cosmux"),
    }
}

fn shim_contents(exe: &Path) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         # forge pod adoption shim — installed by `forge pod adopt` (consolidation RFC §1.5).\n\
         # Redirects `cosmux <args>` to `forge pod <args>` so exactly one binary writes the store.\n\
         # Removed by `forge pod unadopt` / the standalone rollback script.\n\
         exec '{}' pod \"$@\"\n",
        exe.display()
    )
}

#[cfg(test)]
fn assert_shim_write_allowed(path: &Path) {
    let temp = std::env::temp_dir();
    assert!(
        path.starts_with(&temp),
        "REFUSING to install the shim outside the temp dir during tests: {}\n\
         Set {SHIM_PATH_ENV} to a temp path in this test.",
        path.display()
    );
}

#[cfg(not(test))]
fn assert_shim_write_allowed(_path: &Path) {}

fn install_shim(exe: &Path) -> Result<()> {
    let path = shim_path();
    assert_shim_write_allowed(&path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, shim_contents(exe))?;
    set_executable(&path)?;
    Ok(())
}

fn remove_shim() -> Result<()> {
    let path = shim_path();
    assert_shim_write_allowed(&path);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The shim's side-effect is verified, not assumed: present **and** executable.
fn shim_installed() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let path = shim_path();
    match std::fs::metadata(&path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Hook backup — the single restore source, shared by --repair and the standalone script
// ---------------------------------------------------------------------------------------------

pub fn hooks_backup_path() -> PathBuf {
    journal::journal_dir().join("pod-adoption.hooks")
}

/// Which originals to back up, given what the hooks read now and what is already captured — **pure**.
///
/// The rule that keeps rollback un-wedgeable across a resume: back up a hook **only** if it currently
/// reads cosmux (the thing we are about to overwrite) **and** it is not already in the backup. On a
/// resume after a partial rebind, an already-rebound hook reads *forge* — capturing that as the
/// "original" would make rollback restore the hook to forge and then fail its own verify. Because we
/// only ever rebind cosmux-owned hooks, an absent/foreign hook is never touched and so never needs a
/// backup entry — which is why the writer never emits the unset sentinel; [`parse_hook_backup`]
/// still accepts it for forward-compatibility.
pub fn plan_backup(
    current: &[(SessionHook, Option<String>)],
    already: &std::collections::HashSet<String>,
) -> Vec<(SessionHook, String)> {
    current
        .iter()
        .filter_map(|(kind, value)| {
            if already.contains(kind.tmux_name()) {
                return None;
            }
            match classify_hook(value.as_deref()) {
                HookOwner::Cosmux => value.clone().map(|v| (*kind, v)),
                _ => None,
            }
        })
        .collect()
}

/// The set of hook names already captured for a session — makes the backup write-once.
fn backed_up_hooks(session: &str) -> Result<std::collections::HashSet<String>> {
    Ok(read_hook_backup()?
        .into_iter()
        .filter(|r| r.session == session)
        .map(|r| r.hook)
        .collect())
}

/// Append the pre-rebind cosmux hook values for a session so rollback can restore them verbatim.
///
/// TSV (`session \t hook-name \t value`) so the standalone shell script can consume it with
/// `flock + rm + tmux` and no JSON parser. Both this binary's `--repair` and the shell script read
/// the same file — one restore source, never two derivations (advisor C5).
fn append_hook_backup(session: &str, originals: &[(SessionHook, String)]) -> Result<()> {
    use std::io::Write as _;
    let path = hooks_backup_path();
    assert_shim_write_allowed_journal(&path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for (kind, value) in originals {
        // A newline or tab in a hook value would corrupt the TSV; tmux hook strings contain
        // neither, but assert it so a future format change fails loudly instead of silently.
        debug_assert!(!value.contains('\n') && !value.contains('\t'));
        writeln!(file, "{}\t{}\t{}", session, kind.tmux_name(), value)?;
    }
    Ok(())
}

/// One restore instruction parsed from the backup TSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRestore {
    pub session: String,
    pub hook: String,
    /// `None` ⇒ the original was unset ⇒ restore with `set-hook -u`.
    pub value: Option<String>,
}

/// Parse the backup TSV — **pure**, so the restore plan is testable without tmux. Later lines win,
/// matching append order, though in practice each (session, hook) is written once per adoption.
pub fn parse_hook_backup(text: &str) -> Vec<HookRestore> {
    let mut out: Vec<HookRestore> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let (Some(session), Some(hook), Some(value)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let value = if value == UNSET_SENTINEL {
            None
        } else {
            Some(value.to_string())
        };
        out.push(HookRestore {
            session: session.to_string(),
            hook: hook.to_string(),
            value,
        });
    }
    out
}

fn read_hook_backup() -> Result<Vec<HookRestore>> {
    match std::fs::read_to_string(hooks_backup_path()) {
        Ok(text) => Ok(parse_hook_backup(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn remove_hook_backup() -> Result<()> {
    let path = hooks_backup_path();
    assert_shim_write_allowed_journal(&path);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The backup file lives beside the journal, so it inherits the journal's temp-dir write guard.
#[cfg(test)]
fn assert_shim_write_allowed_journal(path: &Path) {
    let temp = std::env::temp_dir();
    assert!(
        path.starts_with(&temp),
        "REFUSING to write the hook backup outside the temp dir during tests: {}",
        path.display()
    );
}

#[cfg(not(test))]
fn assert_shim_write_allowed_journal(_path: &Path) {}

fn hook_name_to_kind(name: &str) -> Option<SessionHook> {
    SessionHook::ALL.into_iter().find(|k| k.tmux_name() == name)
}

// ---------------------------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------------------------

/// `forge pod adopt`. Idempotent: a partial adoption resumes rather than restarting.
pub fn adopt() -> Result<i32> {
    Tmux::ensure_installed()?;
    match journal::authority() {
        Authority::Authorized => {
            println!("already adopted — forge is the production writer");
            return Ok(0);
        }
        Authority::InTransition(TransitionState::Adopting) => {
            println!("resuming an interrupted adoption…");
            return drive_adopt_steps();
        }
        Authority::InTransition(_) => {
            return Err(PodError::Other(anyhow::anyhow!(
                "an unadoption is in progress — run `forge pod unadopt` or `forge pod adopt --abort` first"
            )));
        }
        Authority::Recovery(why) => {
            return Err(PodError::Other(anyhow::anyhow!(
                "adoption journal is in recovery: {why}\nRun `forge pod adopt --repair`."
            )));
        }
        Authority::Unadopted => {}
    }

    // Preflight, THEN write `adopting` before any surface mutation.
    adopt_preflight(cosmux_processes_present(), mid_respawn_panes_present()?)?;
    journal::with_lock(|lock| lock.write(&journal::Journal::new(TransitionState::Adopting)))?;

    drive_adopt_steps()
}

/// The shared adopt/resume body. Every step reconciles the ledger against the real surface, so
/// re-running it after an interruption is safe (`--resume` never trusts the ledger alone).
fn drive_adopt_steps() -> Result<i32> {
    let exe = current_exe();

    if let Err(e) = reconcile_shim_step(&exe) {
        return stop_or_propagate(e, "shim");
    }
    let sessions = Tmux::list_sessions()?;
    for session in &sessions {
        if let Err(e) = reconcile_hook_step(&exe, session) {
            return stop_or_propagate(e, "hooks");
        }
    }
    if let Err(e) = drain_recoveries(&exe) {
        return stop_or_propagate(e, "drain");
    }

    // Terminal commit: re-reads under the lock, loops back to drain if a late recovery landed.
    loop {
        crash_if("terminal/before")?;
        match journal::commit_terminal()? {
            CommitOutcome::Committed => {
                crash_if("terminal/after")?;
                println!("✓ adopted — forge is now the production writer");
                return Ok(0);
            }
            CommitOutcome::PendingRecoveries(_) => {
                drain_recoveries(&exe)?;
                continue;
            }
            CommitOutcome::IncompleteSteps => {
                return Err(PodError::Other(anyhow::anyhow!(
                    "cannot commit: a ledger step is still pending — re-run `forge pod adopt --resume`"
                )));
            }
            CommitOutcome::NotAdopting => {
                return Err(PodError::Other(anyhow::anyhow!(
                    "journal is not in `adopting` — nothing to commit"
                )));
            }
        }
    }
}

/// A modelled crash leaves disk as-is and stops (the operator re-runs `--resume`). A real error
/// propagates.
fn stop_or_propagate(error: PodError, step: &str) -> Result<i32> {
    if is_injected(&error) {
        eprintln!("adoption interrupted during `{step}` ({error}); state is durable — `--resume`");
        return Err(error);
    }
    Err(error)
}

fn reconcile_shim_step(exe: &Path) -> Result<()> {
    crash_if("shim/before-pending")?;
    // Journal::new(Adopting) already carries shim: Pending, so "write pending" is a no-op the first
    // time; on resume it re-asserts pending before re-verifying the surface.
    set_shim_state(StepState::Pending)?;
    crash_if("shim/after-pending")?;

    if !shim_installed() {
        install_shim(exe)?;
    }
    if !shim_installed() {
        return Err(PodError::Other(anyhow::anyhow!(
            "shim install did not produce an executable at {}",
            shim_path().display()
        )));
    }
    crash_if("shim/after-side-effect")?;

    set_shim_state(StepState::Complete)?;
    crash_if("shim/after-complete")?;
    Ok(())
}

fn reconcile_hook_step(exe: &Path, session: &str) -> Result<()> {
    // Read both hooks; a session forge only needs to touch if either is cosmux-owned.
    let current: Vec<(SessionHook, Option<String>)> = SessionHook::ALL
        .into_iter()
        .map(|k| (k, Tmux::show_hook(session, k)))
        .collect();

    let owns_cosmux = current
        .iter()
        .any(|(_, v)| classify_hook(v.as_deref()) == HookOwner::Cosmux);
    let already_forge = current.iter().all(|(_, v)| {
        matches!(
            classify_hook(v.as_deref()),
            HookOwner::Forge | HookOwner::Absent | HookOwner::Foreign
        )
    });

    // Nothing cosmux-owned and not already recorded → not our session, skip with no ledger entry.
    if !owns_cosmux && step_hook_state(session)?.is_none() {
        return Ok(());
    }

    let marker = format!("hooks:{session}");
    crash_if(&format!("{marker}/before-pending"))?;
    set_hook_state(session, StepState::Pending)?;
    crash_if(&format!("{marker}/after-pending"))?;

    // Back up BOTH originals before overwriting, but only once (backup is append-only; a resume
    // must not double-append). We treat a present ledger entry as proof the backup was taken.
    if owns_cosmux {
        // Back up first (write-once, cosmux-only), THEN rebind — so a crash between the two leaves
        // the original recoverable from the backup, never a forge value masquerading as original.
        let already = backed_up_hooks(session)?;
        let to_backup = plan_backup(&current, &already);
        if !to_backup.is_empty() {
            append_hook_backup(session, &to_backup)?;
        }
        for (kind, _) in &current {
            if classify_hook(Tmux::show_hook(session, *kind).as_deref()) == HookOwner::Cosmux {
                Tmux::set_hook(session, *kind, &forge_hook(exe, *kind, session))?;
            }
        }
    }

    // Verify readback: no hook may still point at cosmux (§1 point 4).
    for kind in SessionHook::ALL {
        if classify_hook(Tmux::show_hook(session, kind).as_deref()) == HookOwner::Cosmux {
            return Err(PodError::Other(anyhow::anyhow!(
                "session '{session}' hook {} still points at cosmux after rebind",
                kind.tmux_name()
            )));
        }
    }
    let _ = already_forge;
    crash_if(&format!("{marker}/after-side-effect"))?;

    set_hook_state(session, StepState::Complete)?;
    crash_if(&format!("{marker}/after-complete"))?;
    Ok(())
}

fn drain_recoveries(exe: &Path) -> Result<()> {
    let pending = journal::with_lock(|lock| {
        Ok(lock
            .read()?
            .map(|j| {
                j.pending_recoveries()
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default())
    })?;

    for pane_id in pending {
        // The recovery intent is keyed by session; respawn that session's dead panes inline now
        // that we are draining, then mark complete.
        let session = pane_id.split(':').next().unwrap_or(&pane_id).to_string();
        let _ = exe; // recovery uses forge's own respawn path via recover::pane_recover
        super::recover::pane_recover_inline(&session)?;
        journal::complete_recovery(&pane_id)?;
    }
    Ok(())
}

/// `forge pod unadopt` — the mirror machine. Restores every hook, removes the shim, removes the
/// journal, under the same lock.
pub fn unadopt() -> Result<i32> {
    Tmux::ensure_installed()?;
    match journal::authority() {
        Authority::Unadopted => {
            println!("not adopted — nothing to roll back");
            return Ok(0);
        }
        Authority::Recovery(why) => {
            return Err(PodError::Other(anyhow::anyhow!(
                "journal is in recovery: {why}\nRun `forge pod adopt --repair`."
            )));
        }
        _ => {}
    }
    rollback_surfaces()?;
    println!("✓ unadopted — cosmux is the production writer again");
    Ok(0)
}

/// `forge pod adopt --repair` — deterministic rollback to `unadopted` from any inconsistent journal
/// (§1.5: repair never guesses direction; it reconciles every surface back to cosmux).
pub fn repair() -> Result<i32> {
    Tmux::ensure_installed()?;
    rollback_surfaces()?;
    println!("✓ repaired — rolled back to unadopted; cosmux is the production writer");
    Ok(0)
}

/// `--abort` is repair by another name: reverse every surface, land on `unadopted`.
pub fn abort() -> Result<i32> {
    repair()
}

/// Restore hooks from the backup, remove the shim, remove the backup, remove the journal — the
/// shared teardown for unadopt / --abort / --repair. Restores are verified before the journal is
/// removed, so a failed restore leaves the journal in place to retry rather than declaring success.
fn rollback_surfaces() -> Result<()> {
    // FREEZE FIRST (regate-15 P1-1): write `unadopting` under the lock BEFORE any surface change.
    // Store-write authority is a lock-free journal read (`state::save` → `require_production_write`);
    // if the journal still read `adopted` while we restore cosmux hooks, forge would stay authorized
    // with cosmux hooks already back = a dual-writer window. `unadopting` is `InTransition`, which
    // refuses production writes throughout the rollback. Written unconditionally (not read-modify)
    // so it also freezes a corrupt/unreadable journal, whose direction is deterministically rollback.
    journal::with_lock(|lock| {
        let mut journal = journal::Journal::new(TransitionState::Unadopting);
        journal.touch();
        lock.write(&journal)
    })?;

    let restores = read_hook_backup()?;
    for restore in &restores {
        let Some(kind) = hook_name_to_kind(&restore.hook) else {
            continue;
        };
        match &restore.value {
            Some(value) => Tmux::set_hook(&restore.session, kind, value)?,
            None => Tmux::unset_hook(&restore.session, kind)?,
        }
    }
    // Verify no restored session still shows a forge hook where a cosmux/absent one was expected.
    for restore in &restores {
        let Some(kind) = hook_name_to_kind(&restore.hook) else {
            continue;
        };
        let now = classify_hook(Tmux::show_hook(&restore.session, kind).as_deref());
        if now == HookOwner::Forge {
            return Err(PodError::Other(anyhow::anyhow!(
                "rollback did not restore session '{}' hook {} off forge",
                restore.session,
                restore.hook
            )));
        }
    }

    remove_shim()?;
    remove_hook_backup()?;
    // Journal removal is last: while it exists, authority stays frozen; once gone, `unadopted`.
    journal::with_lock(|lock| lock.remove())?;
    Ok(())
}

// -- journal step mutators (all under the lock) -------------------------------------------------

fn set_shim_state(state: StepState) -> Result<()> {
    journal::with_lock(|lock| {
        if let Some(mut journal) = lock.read()? {
            journal.steps.shim = state;
            journal.touch();
            lock.write(&journal)?;
        }
        Ok(())
    })
}

fn set_hook_state(session: &str, state: StepState) -> Result<()> {
    journal::with_lock(|lock| {
        if let Some(mut journal) = lock.read()? {
            journal.steps.hooks.insert(session.to_string(), state);
            journal.touch();
            lock.write(&journal)?;
        }
        Ok(())
    })
}

fn step_hook_state(session: &str) -> Result<Option<StepState>> {
    journal::with_lock(|lock| {
        Ok(lock
            .read()?
            .and_then(|j| j.steps.hooks.get(session).copied()))
    })
}

// -- environment probes -------------------------------------------------------------------------

/// cosmux is a CLI, not a daemon, so this is normally empty; a hit means someone is mid-invocation.
fn cosmux_processes_present() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "cosmux"])
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

/// Any session with a currently-dead pane is mid-respawn territory.
fn mid_respawn_panes_present() -> Result<bool> {
    for session in Tmux::list_sessions()? {
        let out = Tmux::run(&["list-panes", "-t", &session, "-s", "-F", "#{pane_dead}"]);
        if let Ok(out) = out
            && String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l == "1")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- classify_hook (pure) ------------------------------------------------------------------

    #[test]
    fn classifies_the_three_hook_owners_and_foreign() {
        assert_eq!(classify_hook(None), HookOwner::Absent);
        assert_eq!(classify_hook(Some("  ")), HookOwner::Absent);
        assert_eq!(
            classify_hook(Some(
                "run-shell '/usr/bin/cosmux _pane-recover demo >> /tmp/cosmux-demo.log 2>&1'"
            )),
            HookOwner::Cosmux
        );
        assert_eq!(
            classify_hook(Some(
                "run-shell '/home/x/.local/bin/forge pod _pane-recover demo >> /tmp/forge-pod-demo.log 2>&1'"
            )),
            HookOwner::Forge
        );
        assert_eq!(
            classify_hook(Some("run-shell 'my-own-monitor.sh'")),
            HookOwner::Foreign
        );
    }

    #[test]
    fn a_forge_hook_string_round_trips_to_forge_ownership() {
        // The classifier and the builder must agree, or adopt would rebind its own hooks forever.
        let hook = forge_hook(Path::new("/opt/forge"), SessionHook::PaneDied, "sess");
        assert_eq!(classify_hook(Some(&hook)), HookOwner::Forge);
        let hook = forge_hook(Path::new("/opt/forge"), SessionHook::ClientDetached, "sess");
        assert_eq!(classify_hook(Some(&hook)), HookOwner::Forge);
    }

    // -- preflight (pure) ----------------------------------------------------------------------

    #[test]
    fn preflight_refuses_both_unsafe_conditions_and_passes_when_clear() {
        assert!(adopt_preflight(false, false).is_ok());
        assert!(
            adopt_preflight(true, false)
                .unwrap_err()
                .to_string()
                .contains("cosmux` process")
        );
        assert!(
            adopt_preflight(false, true)
                .unwrap_err()
                .to_string()
                .contains("mid-respawn")
        );
    }

    // -- hook backup parse (pure) --------------------------------------------------------------

    #[test]
    fn hook_backup_round_trips_including_the_unset_sentinel() {
        let text =
            format!("demo\tpane-died\trun-shell \"x\"\ndemo\tclient-detached\t{UNSET_SENTINEL}\n");
        let restores = parse_hook_backup(&text);
        assert_eq!(restores.len(), 2);
        assert_eq!(restores[0].value.as_deref(), Some("run-shell \"x\""));
        assert_eq!(
            restores[1].value, None,
            "the unset sentinel must parse back to a `set-hook -u`"
        );
    }

    #[test]
    fn backup_captures_only_cosmux_originals_never_an_already_rebound_forge_hook() {
        // The resume-after-partial-rebind trap: a real crash between rebinding pane-died and
        // client-detached leaves pane-died reading *forge*. On resume, the backup must NOT record
        // that forge value as the "original" — it would make rollback restore to forge and wedge.
        let cosmux_pd =
            "run-shell '/usr/bin/cosmux _pane-recover s >> /tmp/cosmux-s.log 2>&1'".to_string();
        let forge_pd = forge_hook(Path::new("/opt/forge"), SessionHook::PaneDied, "s");
        let cosmux_cd =
            "run-shell '/usr/bin/cosmux _after-detach s >> /tmp/cosmux-s.log 2>&1'".to_string();

        // First pass: both hooks cosmux, nothing captured yet → both backed up.
        let current = vec![
            (SessionHook::PaneDied, Some(cosmux_pd.clone())),
            (SessionHook::ClientDetached, Some(cosmux_cd.clone())),
        ];
        let plan = plan_backup(&current, &std::collections::HashSet::new());
        assert_eq!(plan.len(), 2);

        // Resume after a partial rebind: pane-died already captured AND now reads forge;
        // client-detached still cosmux. The plan must be exactly client-detached, and pane-died's
        // stored original must stay cosmux.
        let already: std::collections::HashSet<String> = ["pane-died".to_string()].into();
        let current_partial = vec![
            (SessionHook::PaneDied, Some(forge_pd)),
            (SessionHook::ClientDetached, Some(cosmux_cd.clone())),
        ];
        let plan = plan_backup(&current_partial, &already);
        assert_eq!(
            plan.len(),
            1,
            "only the un-captured cosmux hook is backed up"
        );
        assert_eq!(plan[0].0, SessionHook::ClientDetached);
        assert!(
            plan[0].1.contains("/usr/bin/cosmux"),
            "must capture the cosmux original, not forge: {}",
            plan[0].1
        );
    }

    #[test]
    fn hook_backup_skips_blank_and_malformed_lines() {
        let restores = parse_hook_backup("\n\ngarbage-no-tabs\ndemo\tpane-died\tv\n");
        assert_eq!(restores.len(), 1);
        assert_eq!(restores[0].session, "demo");
    }

    // -- injection gating ----------------------------------------------------------------------

    #[test]
    fn injection_is_unreachable_without_a_redirected_journal() {
        use crate::pod::ENV_LOCK;
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_journal = std::env::var_os(journal::JOURNAL_DIR_ENV);
        let prev_fail = std::env::var_os(FAIL_AT_ENV);
        unsafe {
            std::env::remove_var(journal::JOURNAL_DIR_ENV);
            std::env::set_var(FAIL_AT_ENV, "shim/before-pending");
        }
        // No journal redirect ⇒ injection disabled ⇒ the marker is inert.
        assert!(
            crash_if("shim/before-pending").is_ok(),
            "failure injection must be unreachable against the live journal"
        );
        unsafe {
            match prev_journal {
                Some(v) => std::env::set_var(journal::JOURNAL_DIR_ENV, v),
                None => std::env::remove_var(journal::JOURNAL_DIR_ENV),
            }
            match prev_fail {
                Some(v) => std::env::set_var(FAIL_AT_ENV, v),
                None => std::env::remove_var(FAIL_AT_ENV),
            }
        }
    }

    #[test]
    fn shim_contents_execs_forge_pod() {
        let body = shim_contents(Path::new("/opt/forge"));
        assert!(body.starts_with("#!/usr/bin/env bash"));
        assert!(body.contains("exec '/opt/forge' pod \"$@\""));
    }

    #[test]
    #[should_panic(expected = "REFUSING to install the shim outside the temp dir")]
    fn installing_the_live_shim_panics_under_test() {
        assert_shim_write_allowed(Path::new("/home/axw/.local/bin/cosmux"));
    }
}
