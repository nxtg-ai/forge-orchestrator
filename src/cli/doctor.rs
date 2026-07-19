//! `forge doctor` — the IO shell around the pure aggregation in `core::doctor`.
//!
//! Everything that touches the filesystem, git, or the governance checker happens here; the
//! results are injected into `core::doctor::run_doctor`, which stays pure. Resolution failures
//! degrade to a `SKIP` with a stated reason rather than aborting the run — a doctor that cannot
//! report because one probe failed is worse than one that reports what it could establish.

use std::path::Path;
use std::process::Command;

use colored::Colorize;

use crate::brain::rule_based::RuleBasedBrain;
use crate::core::doctor::{DoctorOptions, DoctorReport, run_doctor};
use crate::core::governance::GovernanceChecker;
use crate::core::release_debt::{
    DEFAULT_COMMIT_WARN_THRESHOLD, ReleaseDebtInput, VersionSurface, evaluate,
    parse_cargo_lock_version, parse_cargo_toml_version, parse_json_version,
};

/// Run the doctor and return its fail-closed exit code.
pub fn execute(project_root: &Path, strict: bool, json: bool) -> anyhow::Result<i32> {
    let mut skip_reason = None;

    let release_debt = match collect_release_debt(project_root) {
        Ok(Some(input)) => Some(evaluate(&input)),
        Ok(None) => {
            skip_reason = Some("no recognized version manifest in this project".to_string());
            None
        }
        Err(error) => {
            skip_reason = Some(format!("release debt unresolvable: {error}"));
            None
        }
    };

    // Governance only means something for a forge-INITIALIZED project. `.forge/state.json` is the
    // real signal: a bare `.forge/` directory (or none) means this is simply not a forge project,
    // and reporting "Missing state.json — orchestration state is lost" as a critical health defect
    // would be a category error, not a finding. forge-orchestrator's own repo is exactly this case.
    let initialized = project_root.join(".forge").join("state.json").is_file();
    let governance = if initialized {
        // The rule-based brain is free and offline, so `forge doctor` runs in CI without an API key.
        let checker = GovernanceChecker::new(project_root);
        Some(checker.full_check(&RuleBasedBrain))
    } else {
        None
    };

    let report = run_doctor(&DoctorOptions {
        release_debt,
        governance,
        skip_reason,
        // The rule-based brain cannot measure drift; it returns a neutral score. Claiming that as
        // a drift verdict would be a fabricated green, so the aggregation reports SKIP instead.
        drift_evaluable: false,
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }

    Ok(report.exit_code(strict))
}

/// Outcome of the `forge ship` release preflight.
///
/// Shares the evaluator with `forge doctor` rather than reimplementing it, so the two commands
/// can never disagree about whether a repo is shippable.
pub enum Preflight {
    /// No release debt. Carries a one-line summary.
    Clean(String),
    /// WARN-level debt — surfaced, not blocking. Ship is often what discharges it.
    Warned(Vec<String>),
    /// FAIL-level debt — internally inconsistent versions. Blocks the ship.
    Blocked(Vec<String>),
    /// Nothing to evaluate, with the reason.
    Skipped(String),
}

/// Evaluate release debt for `forge ship`'s preflight step.
pub fn release_preflight(project_root: &Path) -> Preflight {
    let input = match collect_release_debt(project_root) {
        Ok(Some(input)) => input,
        Ok(None) => return Preflight::Skipped("no recognized version manifest".to_string()),
        Err(error) => return Preflight::Skipped(format!("release debt unresolvable: {error}")),
    };

    let report = evaluate(&input);
    let details: Vec<String> = report
        .findings
        .iter()
        .map(|f| format!("{}: {}", f.kind, f.detail))
        .collect();

    match report.status.as_str() {
        "FAIL" => Preflight::Blocked(details),
        "WARN" => Preflight::Warned(details),
        _ => Preflight::Clean(format!(
            "version {} agrees across {} surface(s); tag {}",
            report.version.as_deref().unwrap_or("unknown"),
            input.surfaces.len(),
            report.latest_tag.as_deref().unwrap_or("none")
        )),
    }
}

fn print_report(report: &DoctorReport) {
    let header = match report.overall.as_str() {
        "FAIL" => "FORGE DOCTOR: FAIL".red().bold(),
        "WARN" => "FORGE DOCTOR: WARN".yellow().bold(),
        _ => "FORGE DOCTOR: OK".green().bold(),
    };
    println!(
        "{header} | ok={} warn={} fail={} skip={}",
        report.ok, report.warn, report.fail, report.skip
    );

    for check in &report.checks {
        let tag = match check.status.as_str() {
            "FAIL" => "[FAIL]".red().bold(),
            "WARN" => "[WARN]".yellow().bold(),
            "SKIP" => "[SKIP]".dimmed(),
            _ => "[ OK ]".green(),
        };
        println!("  {tag} {} — {}", check.name.bold(), check.detail);
    }
}

/// Discover this project's version surfaces and git release state.
///
/// Returns `Ok(None)` when the directory holds no recognized manifest — that is a legitimate
/// "nothing to check", distinct from a probe that failed.
fn collect_release_debt(root: &Path) -> anyhow::Result<Option<ReleaseDebtInput>> {
    let surfaces = collect_version_surfaces(root)?;
    if surfaces.is_empty() {
        return Ok(None);
    }

    let latest_tag = git_latest_tag(root);
    let commits_since_tag = git_commit_count(root, latest_tag.as_deref());

    Ok(Some(ReleaseDebtInput {
        surfaces,
        latest_tag,
        commits_since_tag,
        commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
    }))
}

/// Every file in this repo that declares a version.
///
/// Deliberately a *set*, not a single detected manifest: a repo whose manifests disagree with
/// each other is broken, and a single-manifest probe reports it as healthy.
fn collect_version_surfaces(root: &Path) -> anyhow::Result<Vec<VersionSurface>> {
    let mut surfaces = Vec::new();

    // Rust: Cargo.toml + its lockfile entry for the same package.
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.is_file() {
        let text = std::fs::read_to_string(&cargo_toml)?;
        let version = parse_cargo_toml_version(&text);
        surfaces.push(VersionSurface::manifest("Cargo.toml", version.clone()));

        let cargo_lock = root.join("Cargo.lock");
        if cargo_lock.is_file()
            && let Some(package_name) = parse_cargo_package_name(&text)
        {
            let lock_text = std::fs::read_to_string(&cargo_lock)?;
            surfaces.push(VersionSurface::lockfile(
                "Cargo.lock",
                parse_cargo_lock_version(&lock_text, &package_name),
            ));
        }
    }

    // Node: package.json + package-lock.json.
    for (path, is_lock) in [
        ("package.json", false),
        ("package-lock.json", true),
        (".claude-plugin/plugin.json", false),
        (".claude-plugin/marketplace.json", false),
    ] {
        let candidate = root.join(path);
        if candidate.is_file() {
            let version = parse_json_version(&std::fs::read_to_string(&candidate)?);
            surfaces.push(if is_lock {
                VersionSurface::lockfile(path, version)
            } else {
                VersionSurface::manifest(path, version)
            });
        }
    }

    Ok(surfaces)
}

/// Read `name` from a `Cargo.toml` `[package]` section, so the right lock block is selected.
fn parse_cargo_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = trimmed.strip_prefix("name")
            && let Some(rest) = rest.trim_start().strip_prefix('=')
            && let Some(rest) = rest.trim().strip_prefix('"')
            && let Some(end) = rest.find('"')
        {
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn git_latest_tag(root: &Path) -> Option<String> {
    run_git(root, &["describe", "--tags", "--abbrev=0"])
}

fn git_commit_count(root: &Path, tag: Option<&str>) -> u32 {
    let range = match tag {
        Some(tag) => format!("{tag}..HEAD"),
        None => "HEAD".to_string(),
    };
    run_git(root, &["rev-list", &range, "--count"])
        .and_then(|out| out.parse().ok())
        .unwrap_or(0)
}

/// Run a git command, returning trimmed stdout on success.
///
/// Any failure — git absent, not a repo, no tags yet — yields `None`. The caller turns that into
/// a visible `SKIP`/`untagged` finding rather than an error, so `forge doctor` still works in a
/// freshly-created directory.
fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cargo_package_name() {
        let text = "[package]\nname = \"forge-orchestrator\"\nversion = \"1.5.2\"\n";
        assert_eq!(
            parse_cargo_package_name(text).as_deref(),
            Some("forge-orchestrator")
        );
    }

    #[test]
    fn package_name_ignores_dependency_names() {
        let text = "[dependencies]\nname = \"wrong\"\n\n[package]\nname = \"right\"\n";
        assert_eq!(parse_cargo_package_name(text).as_deref(), Some("right"));
    }

    #[test]
    fn empty_directory_yields_no_surfaces() {
        let dir = std::env::temp_dir().join(format!("forge-doctor-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let surfaces = collect_version_surfaces(&dir).unwrap();
        assert!(surfaces.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn this_repo_reports_agreeing_cargo_surfaces() {
        // Runs against the real repo: Cargo.toml and Cargo.lock must agree, which is exactly
        // the invariant the v1.5.1 incident violated.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let surfaces = collect_version_surfaces(root).unwrap();
        let manifest = surfaces
            .iter()
            .find(|s| s.path == "Cargo.toml")
            .expect("Cargo.toml surface");
        let lock = surfaces
            .iter()
            .find(|s| s.path == "Cargo.lock")
            .expect("Cargo.lock surface");
        assert!(manifest.version.is_some(), "Cargo.toml version must parse");
        assert_eq!(
            manifest.version, lock.version,
            "Cargo.toml and Cargo.lock must agree — this is the v1.5.1 desync guard"
        );
    }
}
