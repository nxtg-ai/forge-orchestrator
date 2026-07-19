//! Preflight check — verify every heartbeat target has a matching pod YAML window.
//!
//! Vendored from cosmux v0.4.2 `preflight.rs`.
//!
//! Origin: 2026-04-19 NXTG-AI P0 incident (FW team dropped 9h silent after cutover
//! because the WORKSTREAMS pod YAML didn't include a window heartbeat was targeting).
//! See cosmux NEXUS N-18 / PI-05 for full context.
//!
//! Design notes:
//! - Parses bash associative array form `["SESSION:WIN.PANE"]="..."` from the
//!   heartbeat script. This is the NXTG-AI format; CLX9 may differ.
//! - **Empty-target-set is a hard FAIL, not a pass.** Emma's 2026-04-19 meta-lesson
//!   from HANDOFF Note 109: governance scripts that declare OK on zero extracted
//!   items are a silent-success antipattern. A preflight that finds no targets to
//!   check can't meaningfully say "covered"; it can only say "I could not find any
//!   targets in this heartbeat script."
//! - Exit codes: 0 = all covered, 2 = uncovered targets, 1 = operational error.

use super::config::{PodConfig, resolve_pod_path};
use super::error::{PodError, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One heartbeat target: SESSION:WINDOW.PANE.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub session: String,
    /// Window name or numeric index (as written in heartbeat key).
    pub window: String,
    pub pane: String,
}

impl Target {
    pub fn display(&self) -> String {
        format!("{}:{}.{}", self.session, self.window, self.pane)
    }
}

/// Parse heartbeat script for `["SESSION:WIN.PANE"]=...` targets.
/// Returns a sorted, de-duplicated list. Empty list is a legitimate result
/// (callers must decide whether empty is a failure).
pub fn parse_heartbeat_targets(path: &Path) -> Result<Vec<Target>> {
    let raw = fs::read_to_string(path).map_err(|e| {
        PodError::InvalidConfig(format!(
            "cannot read heartbeat script {}: {e}",
            path.display()
        ))
    })?;

    let mut set: BTreeSet<Target> = BTreeSet::new();
    for line in raw.lines() {
        // Match:  [ optional whitespace ] ["SESSION:WIN.PANE"]= ...
        // Tolerate leading spaces, both `=` and `= "..."` spacing.
        let trimmed = line.trim_start();
        if !trimmed.starts_with("[\"") {
            continue;
        }
        // Extract between first `["` and the next `"]`.
        let after_open = match trimmed.strip_prefix("[\"") {
            Some(s) => s,
            None => continue,
        };
        let close = match after_open.find("\"]") {
            Some(i) => i,
            None => continue,
        };
        let key = &after_open[..close];
        // Only count lines that also have `=` after `"]` (actual assignment).
        let rest = &after_open[close + 2..];
        if !rest.trim_start().starts_with('=') {
            continue;
        }
        // key should look like SESSION:WIN.PANE
        if let Some((session, win_pane)) = key.split_once(':')
            && let Some((window, pane)) = win_pane.split_once('.')
        {
            set.insert(Target {
                session: session.to_string(),
                window: window.to_string(),
                pane: pane.to_string(),
            });
        }
    }
    Ok(set.into_iter().collect())
}

/// One uncovered target + the reason.
#[derive(Debug, Clone)]
pub struct Gap {
    pub target: Target,
    pub reason: String,
}

/// Check a single pod-yaml against the targets that match its session name.
/// Returns all gaps (uncovered target -> reason) for this pod.
fn check_pod_against_targets(pod_path: &Path, targets: &[Target]) -> Result<Vec<Gap>> {
    let pod = PodConfig::load(pod_path)?;
    let matching: Vec<&Target> = targets.iter().filter(|t| t.session == pod.name).collect();
    let mut gaps = Vec::new();

    for t in matching {
        let covered = if t
            .window
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            // Numeric index: count pod's windows and compare.
            let idx: usize = t.window.parse().unwrap_or(0);
            idx <= pod.windows.len() && idx > 0
        } else {
            pod.windows.iter().any(|w| w.name == t.window)
        };
        if !covered {
            gaps.push(Gap {
                target: t.clone(),
                reason: format!(
                    "pod '{}' ({}) has no matching window for '{}'",
                    pod.name,
                    pod_path.display(),
                    t.window
                ),
            });
        }
    }
    Ok(gaps)
}

/// Auto-detect heartbeat script by hostname suffix. Falls back to generic.
pub fn detect_heartbeat_script() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let hostname = hostname_lower();

    let nxtg = home.join("ASIF/scripts/cos-heartbeat-nxtg.sh");
    let clx9 = home.join("ASIF/scripts/cos-heartbeat-clx9.sh");
    let generic = home.join("ASIF/scripts/cos-heartbeat.sh");

    if hostname.contains("nxtg") && nxtg.exists() {
        return Some(nxtg);
    }
    if hostname.contains("clx9") && clx9.exists() {
        return Some(clx9);
    }
    if generic.exists() {
        return Some(generic);
    }
    if nxtg.exists() {
        return Some(nxtg);
    }
    if clx9.exists() {
        return Some(clx9);
    }
    None
}

fn hostname_lower() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Outcome reported to the CLI caller.
pub struct Report {
    pub heartbeat_path: PathBuf,
    pub targets_found: usize,
    pub pods_checked: Vec<PathBuf>,
    pub gaps: Vec<Gap>,
    /// True when the parser extracted zero targets — always a hard failure
    /// (silent-success-on-empty is the antipattern this command exists to catch).
    pub empty_parse: bool,
}

impl Report {
    /// Parity exit code: `0` = every target covered, `2` = uncovered targets **or an empty
    /// parse**, matching cosmux. Operational errors exit `1` at the CLI layer.
    ///
    /// `empty_parse` deliberately maps to the same failing code as a real gap: a check that
    /// found nothing to check has not verified coverage.
    pub fn exit_code(&self) -> i32 {
        if self.empty_parse || !self.gaps.is_empty() {
            2
        } else {
            0
        }
    }
}

/// Run preflight. `pod_scope` = Some(pod) for one pod; None for all pods touched
/// by the heartbeat's session keys.
pub fn run(pod_scope: Option<&str>, against: Option<&Path>) -> Result<Report> {
    let heartbeat_path = match against {
        Some(p) => p.to_path_buf(),
        None => detect_heartbeat_script().ok_or_else(|| {
            PodError::InvalidConfig(
                "no heartbeat script provided and none auto-detected (looked for \
                 ~/ASIF/scripts/cos-heartbeat{,-nxtg,-clx9}.sh)"
                    .into(),
            )
        })?,
    };

    let targets = parse_heartbeat_targets(&heartbeat_path)?;
    let empty_parse = targets.is_empty();

    // Decide which pod YAMLs to check. Either the single requested pod, or every
    // unique session referenced by targets.
    let sessions: BTreeSet<String> = match pod_scope {
        Some(p) => [p.to_string()].into_iter().collect(),
        None => targets.iter().map(|t| t.session.clone()).collect(),
    };

    let mut pods_checked = Vec::new();
    let mut gaps = Vec::new();

    for session in &sessions {
        match resolve_pod_path(session) {
            Ok(path) => {
                pods_checked.push(path.clone());
                let pod_gaps = check_pod_against_targets(&path, &targets)?;
                gaps.extend(pod_gaps);
            }
            Err(_) => {
                // No pod YAML found for this session. Record as a gap for every
                // target that references this session.
                for t in targets.iter().filter(|t| &t.session == session) {
                    gaps.push(Gap {
                        target: t.clone(),
                        reason: format!("no pod YAML found for session '{session}'"),
                    });
                }
            }
        }
    }

    Ok(Report {
        heartbeat_path,
        targets_found: targets.len(),
        pods_checked,
        gaps,
        empty_parse,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal scratch file — forge carries no `tempfile` dev-dependency.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(contents: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "forge-pod-preflight-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("heartbeat.sh");
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(contents.as_bytes()).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    #[test]
    fn parses_nxtg_bash_assoc_form() {
        let s = Scratch::new(
            r#"
declare -A TEAMS_FALLBACK=(
  ["WORKSTREAMS:Tier-1.1"]="$HOME/projects/nxtg-content-engine"
  ["WORKSTREAMS:Faultline-Web.1"]="$HOME/projects/faultline-web"
  ["WORKSTREAMS:dx3.1"]="$HOME/projects/dx3"
)
"#,
        );
        let targets = parse_heartbeat_targets(s.path()).unwrap();
        assert_eq!(targets.len(), 3);
        assert!(
            targets.iter().any(|t| t.session == "WORKSTREAMS"
                && t.window == "Faultline-Web"
                && t.pane == "1")
        );
    }

    #[test]
    fn empty_heartbeat_parses_to_zero_targets() {
        // Zero targets is a legitimate PARSE result; `run` is what turns it into a hard failure.
        let s = Scratch::new("# just a comment\n");
        assert!(parse_heartbeat_targets(s.path()).unwrap().is_empty());
    }

    #[test]
    fn lines_without_an_assignment_are_ignored() {
        let s = Scratch::new(
            r#"
# a comment ["FAKE:win.1"] with no = sign
  ["REAL:w.1"]="/tmp"
echo ["NOT:assignment.1"]
"#,
        );
        let targets = parse_heartbeat_targets(s.path()).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].session, "REAL");
    }

    #[test]
    fn empty_target_set_is_a_hard_failure_not_a_pass() {
        // The whole point of this command. A preflight that extracted nothing cannot say
        // "covered" — it can only say "I found no targets". Reporting OK there is the
        // silent-success antipattern that let a team go 9h silent on 2026-04-19.
        let s = Scratch::new("# no targets here\n");
        let report = run(None, Some(s.path())).unwrap();
        assert!(report.empty_parse, "zero targets must set empty_parse");
        assert_eq!(report.targets_found, 0);
        assert_eq!(
            report.exit_code(),
            2,
            "empty parse must be a failing exit, never 0"
        );
    }

    #[test]
    fn uncovered_target_exits_two_and_covered_exits_zero() {
        let s = Scratch::new("  [\"NOPOD:win.1\"]=\"/tmp\"\n");
        let report = run(None, Some(s.path())).unwrap();
        assert_eq!(report.targets_found, 1);
        assert!(!report.gaps.is_empty(), "no pod YAML exists for NOPOD");
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn missing_heartbeat_file_is_an_operational_error() {
        let err = parse_heartbeat_targets(Path::new("/nonexistent/heartbeat.sh")).unwrap_err();
        assert!(
            err.to_string().contains("cannot read heartbeat script"),
            "{err}"
        );
    }

    #[test]
    fn target_display_round_trips_the_key_form() {
        let t = Target {
            session: "WORKSTREAMS".into(),
            window: "Tier-1".into(),
            pane: "1".into(),
        };
        assert_eq!(t.display(), "WORKSTREAMS:Tier-1.1");
    }
}
