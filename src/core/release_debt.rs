//! Release-debt evaluation — pure comparison logic, no IO.
//!
//! Ports the *patterns* from `asifctl`'s release evaluator (commits-since-tag, tag↔manifest
//! drift) and closes the two gaps that make a literal port insufficient for this program:
//!
//! 1. **Lockfile agreement.** `asifctl`'s `detect_manifest` selects a single manifest and never
//!    reads a lockfile, so it cannot see the class of defect that actually bit this repo: at
//!    commit `eaf8532`, `Cargo.toml` read `1.5.1` while `Cargo.lock` still read `1.5.0`
//!    (corrected one commit later in `e319616`). A `--locked` build fails on that state.
//! 2. **Multi-surface projects.** A repo's version can live on several files at once —
//!    forge-plugin carries five. Agreement across *all* of them is the contract; checking one
//!    manifest passes a repo that is internally inconsistent.
//!
//! Everything here takes values and returns verdicts. Reading files and shelling out to git
//! happens in `cli::doctor`, which injects the results. That split is what lets the tests below
//! exercise every defect class with string literals — no temp dirs, no git checkouts.

use serde::{Deserialize, Serialize};

/// How many commits may accumulate past the latest tag before it is worth flagging.
///
/// Matches the FPL release-discipline threshold: >5 unreleased commits is a P1 signal.
pub const DEFAULT_COMMIT_WARN_THRESHOLD: u32 = 5;

/// A single file that declares the project's version.
///
/// `path` is informational — it is echoed in findings so a human knows which file to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionSurface {
    /// Repo-relative path, e.g. `Cargo.toml` or `.claude-plugin/plugin.json`.
    pub path: String,
    /// The version string read from that file, if one was found.
    pub version: Option<String>,
    /// True when this surface is a lockfile (`Cargo.lock`, `package-lock.json`).
    ///
    /// Lockfile disagreement is reported as its own finding because it has a distinct
    /// consequence — a `--locked` / `npm ci` build fails outright, rather than merely shipping
    /// an inconsistent number.
    pub is_lockfile: bool,
}

impl VersionSurface {
    /// A manifest surface (`Cargo.toml`, `package.json`, `plugin.json`, …).
    pub fn manifest(path: impl Into<String>, version: Option<String>) -> Self {
        Self {
            path: path.into(),
            version,
            is_lockfile: false,
        }
    }

    /// A lockfile surface (`Cargo.lock`, `package-lock.json`).
    pub fn lockfile(path: impl Into<String>, version: Option<String>) -> Self {
        Self {
            path: path.into(),
            version,
            is_lockfile: true,
        }
    }
}

/// Everything the evaluator needs, already resolved by the caller.
#[derive(Debug, Clone, Default)]
pub struct ReleaseDebtInput {
    /// Every file that declares this project's version. The first non-lockfile surface is
    /// treated as authoritative when reporting drift.
    pub surfaces: Vec<VersionSurface>,
    /// Latest release tag, e.g. `v1.5.2`. `None` when the repo has never been tagged.
    pub latest_tag: Option<String>,
    /// Commits between `latest_tag` and HEAD.
    pub commits_since_tag: u32,
    /// Threshold above which `commits_since_tag` becomes a WARN.
    pub commit_warn_threshold: u32,
}

/// One problem found. `status` is `WARN` or `FAIL`; `OK` never produces a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebtFinding {
    pub kind: String,
    pub status: String,
    pub detail: String,
}

/// The evaluator's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseDebtReport {
    /// `OK`, `WARN`, or `FAIL`.
    pub status: String,
    /// The version agreed on by the manifest surfaces, when they agree.
    pub version: Option<String>,
    pub latest_tag: Option<String>,
    pub commits_since_tag: u32,
    pub findings: Vec<DebtFinding>,
}

fn finding(kind: &str, status: &str, detail: String) -> DebtFinding {
    DebtFinding {
        kind: kind.to_string(),
        status: status.to_string(),
        detail,
    }
}

/// Strip a leading `v` so `v1.5.2` and `1.5.2` compare equal.
fn normalize_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Evaluate release debt. Pure: same input always yields the same report.
///
/// Severity rationale — the two failure classes are not equally bad:
///
/// * **FAIL** for internal inconsistency: manifest surfaces disagreeing with each other, or a
///   lockfile disagreeing with its manifest. These are broken *now* — a `--locked` build fails,
///   or the repo ships two different version numbers.
/// * **WARN** for accumulated debt: unreleased commits, or a tag behind the manifest. The repo
///   is coherent; it just has not shipped. `--strict` is what turns that into a blocking exit.
pub fn evaluate(input: &ReleaseDebtInput) -> ReleaseDebtReport {
    let mut findings = Vec::new();

    let (manifests, lockfiles): (Vec<_>, Vec<_>) =
        input.surfaces.iter().partition(|s| !s.is_lockfile);

    // A surface that exists but has no readable version is a defect in its own right —
    // silently treating it as "agrees" is how a missing version slips through.
    for surface in &input.surfaces {
        if surface.version.is_none() {
            findings.push(finding(
                "version-unreadable",
                "FAIL",
                format!("{}: no version field found", surface.path),
            ));
        }
    }

    // 1. Manifest agreement across every declaring surface (the multi-surface requirement).
    let mut manifest_versions: Vec<(&str, &str)> = manifests
        .iter()
        .filter_map(|s| s.version.as_deref().map(|v| (s.path.as_str(), v)))
        .collect();
    manifest_versions.sort_by(|a, b| a.0.cmp(b.0));

    let mut distinct: Vec<&str> = manifest_versions.iter().map(|(_, v)| *v).collect();
    distinct.sort_unstable();
    distinct.dedup();

    let agreed_version = match distinct.len() {
        0 => None,
        1 => Some(distinct[0].to_string()),
        _ => {
            let detail = manifest_versions
                .iter()
                .map(|(path, version)| format!("{path}={version}"))
                .collect::<Vec<_>>()
                .join(", ");
            findings.push(finding(
                "multi-surface-drift",
                "FAIL",
                format!("version surfaces disagree: {detail}"),
            ));
            None
        }
    };

    // 2. Lockfile agreement. Compared against the agreed manifest version; when the manifests
    //    themselves disagree we skip this rather than emit a cascade of derived findings —
    //    the manifest drift above is the actionable root cause.
    if let Some(expected) = agreed_version.as_deref() {
        for lock in &lockfiles {
            if let Some(actual) = lock.version.as_deref()
                && actual != expected
            {
                findings.push(finding(
                    "lockfile-desync",
                    "FAIL",
                    format!(
                        "{} declares {} but manifest declares {} — a --locked build fails on this state",
                        lock.path, actual, expected
                    ),
                ));
            }
        }
    }

    // 3. Tag ↔ manifest drift.
    match (&input.latest_tag, agreed_version.as_deref()) {
        (Some(tag), Some(version)) if normalize_tag(tag) != version => {
            findings.push(finding(
                "tag-drift",
                "WARN",
                format!("manifest is {version} but latest tag is {tag} — release not cut"),
            ));
        }
        (None, Some(version)) => {
            findings.push(finding(
                "untagged",
                "WARN",
                format!("manifest is {version} but the repo has no release tag"),
            ));
        }
        _ => {}
    }

    // 4. Unreleased commit accumulation.
    if input.commits_since_tag > input.commit_warn_threshold {
        findings.push(finding(
            "unreleased-commits",
            "WARN",
            format!(
                "{} commits since {} (threshold {})",
                input.commits_since_tag,
                input.latest_tag.as_deref().unwrap_or("the initial commit"),
                input.commit_warn_threshold
            ),
        ));
    }

    let status = if findings.iter().any(|f| f.status == "FAIL") {
        "FAIL"
    } else if findings.iter().any(|f| f.status == "WARN") {
        "WARN"
    } else {
        "OK"
    };

    ReleaseDebtReport {
        status: status.to_string(),
        version: agreed_version,
        latest_tag: input.latest_tag.clone(),
        commits_since_tag: input.commits_since_tag,
        findings,
    }
}

/// Extract `version` from a `[package]` section without a TOML parser.
///
/// The no-new-runtime-deps constraint rules out the `toml` crate, so this is deliberately
/// narrow: it finds the `[package]` table and returns the first `version = "..."` inside it,
/// stopping at the next section header. It is not a general TOML reader and does not try to be.
pub fn parse_cargo_toml_version(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // Any new section ends [package]; `[package.metadata]` is a subtable, not the end.
            in_package = trimmed == "[package]" || trimmed.starts_with("[package.");
            if trimmed == "[package]" {
                in_package = true;
                continue;
            }
        }
        if in_package && let Some(value) = parse_toml_string_value(trimmed, "version") {
            return Some(value);
        }
    }
    None
}

/// Extract a package's version from `Cargo.lock`.
///
/// Locates the `[[package]]` block whose `name` matches, then reads *that block's* `version`.
/// Scanning for the first `version` after the name would be wrong: `Cargo.lock` blocks may
/// carry `source`/`checksum`/`dependencies` lines, and a `dependencies` list runs into the next
/// block — so a naive scan silently returns a different package's version.
pub fn parse_cargo_lock_version(text: &str, package_name: &str) -> Option<String> {
    let mut in_target_block = false;
    let mut block_version: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed == "[[package]]" {
            // Entering a new block: whatever we were tracking is finished.
            if in_target_block {
                return block_version;
            }
            in_target_block = false;
            block_version = None;
            continue;
        }

        if let Some(name) = parse_toml_string_value(trimmed, "name") {
            in_target_block = name == package_name;
            continue;
        }

        if in_target_block && let Some(version) = parse_toml_string_value(trimmed, "version") {
            block_version = Some(version);
        }
    }

    if in_target_block { block_version } else { None }
}

/// Parse `key = "value"` from a single trimmed TOML line.
fn parse_toml_string_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Extract a top-level `"version"` from JSON (`package.json`, `plugin.json`, `package-lock.json`).
pub fn parse_json_version(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value
        .get("version")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

/// Extract `(plugin_name, version)` pairs from a Claude Code `marketplace.json`.
///
/// A marketplace manifest has **no top-level `version`** — each entry in `plugins[]` carries its
/// own. Reading it with [`parse_json_version`] yields `None`, which the evaluator would report as
/// `version-unreadable`: a **false FAIL on a perfectly clean checkout**. That is not a hypothetical
/// — forge-plugin's `.claude-plugin/marketplace.json` is exactly this shape, and it is why this
/// function exists rather than a generic JSON reader.
pub fn parse_marketplace_versions(text: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(plugins) = value.get("plugins").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    plugins
        .iter()
        .filter_map(|entry| {
            let version = entry.get("version")?.as_str()?.to_string();
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("<unnamed>")
                .to_string();
            Some((name, version))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(surfaces: Vec<VersionSurface>) -> ReleaseDebtInput {
        ReleaseDebtInput {
            surfaces,
            latest_tag: Some("v1.5.2".into()),
            commits_since_tag: 0,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        }
    }

    #[test]
    fn clean_repo_is_ok() {
        let report = evaluate(&input(vec![
            VersionSurface::manifest("Cargo.toml", Some("1.5.2".into())),
            VersionSurface::lockfile("Cargo.lock", Some("1.5.2".into())),
        ]));
        assert_eq!(report.status, "OK");
        assert!(report.findings.is_empty());
        assert_eq!(report.version.as_deref(), Some("1.5.2"));
    }

    /// The real incident: at `eaf8532` the bump landed in Cargo.toml but Cargo.lock still
    /// carried the previous version; `e319616` fixed it one commit later. Verified against
    /// this repo's history with `git show <sha>:Cargo.toml` / `:Cargo.lock`.
    #[test]
    fn v1_5_1_cargo_lock_desync_is_detected() {
        let report = evaluate(&ReleaseDebtInput {
            surfaces: vec![
                VersionSurface::manifest("Cargo.toml", Some("1.5.1".into())),
                VersionSurface::lockfile("Cargo.lock", Some("1.5.0".into())),
            ],
            latest_tag: Some("v1.5.0".into()),
            commits_since_tag: 2,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        });

        assert_eq!(report.status, "FAIL");
        let desync = report
            .findings
            .iter()
            .find(|f| f.kind == "lockfile-desync")
            .expect("lockfile desync must be reported");
        assert_eq!(desync.status, "FAIL");
        assert!(desync.detail.contains("1.5.0"), "{}", desync.detail);
        assert!(desync.detail.contains("1.5.1"), "{}", desync.detail);
    }

    #[test]
    fn multi_surface_disagreement_fails() {
        // The forge-plugin shape: five surfaces, one of them stale.
        let report = evaluate(&input(vec![
            VersionSurface::manifest(".claude-plugin/plugin.json", Some("3.10.2".into())),
            VersionSurface::manifest(".claude-plugin/marketplace.json", Some("3.10.2".into())),
            VersionSurface::manifest(
                "plugins/nxtg-forge/.claude-plugin/plugin.json",
                Some("3.10.2".into()),
            ),
            VersionSurface::manifest(
                "plugins/nxtg-forge/servers/governance-mcp/package.json",
                Some("3.7.0".into()),
            ),
            VersionSurface::lockfile(
                "plugins/nxtg-forge/servers/governance-mcp/package-lock.json",
                Some("3.10.2".into()),
            ),
        ]));

        assert_eq!(report.status, "FAIL");
        let drift = report
            .findings
            .iter()
            .find(|f| f.kind == "multi-surface-drift")
            .expect("multi-surface drift must be reported");
        assert!(drift.detail.contains("3.7.0"), "{}", drift.detail);
        assert!(drift.detail.contains("package.json"), "{}", drift.detail);
        // Manifests disagree, so no single expected version exists to judge the lockfile by.
        assert!(report.version.is_none());
    }

    #[test]
    fn single_manifest_port_would_pass_the_plugin_case() {
        // Guards Codex finding 5: checking only the first manifest sees 3.10.2 and reports OK,
        // which is exactly the false pass this evaluator exists to prevent.
        let surfaces = vec![
            VersionSurface::manifest(".claude-plugin/plugin.json", Some("3.10.2".into())),
            VersionSurface::manifest("governance-mcp/package.json", Some("3.7.0".into())),
        ];
        let single_surface_only = evaluate(&ReleaseDebtInput {
            surfaces: vec![surfaces[0].clone()],
            latest_tag: Some("v3.10.2".into()),
            commits_since_tag: 0,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        });
        assert_eq!(single_surface_only.status, "OK");

        let all_surfaces = evaluate(&ReleaseDebtInput {
            surfaces,
            latest_tag: Some("v3.10.2".into()),
            commits_since_tag: 0,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        });
        assert_eq!(all_surfaces.status, "FAIL");
    }

    #[test]
    fn tag_drift_warns() {
        let report = evaluate(&ReleaseDebtInput {
            surfaces: vec![VersionSurface::manifest("Cargo.toml", Some("1.6.0".into()))],
            latest_tag: Some("v1.5.2".into()),
            commits_since_tag: 1,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        });
        assert_eq!(report.status, "WARN");
        assert!(report.findings.iter().any(|f| f.kind == "tag-drift"));
    }

    #[test]
    fn unreleased_commits_warn_above_threshold() {
        let mut base = input(vec![VersionSurface::manifest(
            "Cargo.toml",
            Some("1.5.2".into()),
        )]);
        base.commits_since_tag = 6;
        let report = evaluate(&base);
        assert_eq!(report.status, "WARN");
        let f = report
            .findings
            .iter()
            .find(|f| f.kind == "unreleased-commits")
            .expect("unreleased commits must be reported");
        assert!(f.detail.contains('6'), "{}", f.detail);

        base.commits_since_tag = 5;
        assert_eq!(evaluate(&base).status, "OK", "threshold is exclusive");
    }

    #[test]
    fn untagged_repo_warns() {
        let report = evaluate(&ReleaseDebtInput {
            surfaces: vec![VersionSurface::manifest("Cargo.toml", Some("0.1.0".into()))],
            latest_tag: None,
            commits_since_tag: 3,
            commit_warn_threshold: DEFAULT_COMMIT_WARN_THRESHOLD,
        });
        assert_eq!(report.status, "WARN");
        assert!(report.findings.iter().any(|f| f.kind == "untagged"));
    }

    #[test]
    fn unreadable_version_fails() {
        let report = evaluate(&input(vec![
            VersionSurface::manifest("Cargo.toml", Some("1.5.2".into())),
            VersionSurface::manifest("plugin.json", None),
        ]));
        assert_eq!(report.status, "FAIL");
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == "version-unreadable")
        );
    }

    #[test]
    fn tag_v_prefix_is_normalized() {
        let report = evaluate(&input(vec![VersionSurface::manifest(
            "Cargo.toml",
            Some("1.5.2".into()),
        )]));
        assert_eq!(report.status, "OK", "v1.5.2 must equal 1.5.2");
    }

    #[test]
    fn parses_cargo_toml_package_version() {
        let text = r#"
[package]
name = "forge-orchestrator"
version = "1.5.2"
edition = "2024"

[dependencies]
clap = { version = "4", features = ["derive"] }
"#;
        assert_eq!(parse_cargo_toml_version(text).as_deref(), Some("1.5.2"));
    }

    #[test]
    fn cargo_toml_parser_ignores_dependency_versions() {
        // A dependency version appearing before [package] must not be mistaken for the crate's.
        let text = r#"
[workspace]

[dependencies]
serde = { version = "9.9.9" }

[package]
name = "forge-orchestrator"
version = "1.5.2"
"#;
        assert_eq!(parse_cargo_toml_version(text).as_deref(), Some("1.5.2"));
    }

    #[test]
    fn parses_cargo_lock_block_for_the_named_package() {
        let text = r#"
[[package]]
name = "clap"
version = "4.5.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "clap_builder",
]

[[package]]
name = "forge-orchestrator"
version = "1.5.2"
dependencies = [
 "anyhow",
 "clap",
]

[[package]]
name = "serde"
version = "1.0.200"
"#;
        assert_eq!(
            parse_cargo_lock_version(text, "forge-orchestrator").as_deref(),
            Some("1.5.2")
        );
        assert_eq!(
            parse_cargo_lock_version(text, "clap").as_deref(),
            Some("4.5.0")
        );
        assert_eq!(
            parse_cargo_lock_version(text, "serde").as_deref(),
            Some("1.0.200")
        );
        assert_eq!(parse_cargo_lock_version(text, "absent"), None);
    }

    #[test]
    fn cargo_lock_parser_does_not_leak_into_the_next_block() {
        // The naive "first version line after the name" approach returns 9.9.9 here, because
        // the target block has no version of its own. It must return None instead.
        let text = r#"
[[package]]
name = "forge-orchestrator"
dependencies = [
 "anyhow",
]

[[package]]
name = "other"
version = "9.9.9"
"#;
        assert_eq!(parse_cargo_lock_version(text, "forge-orchestrator"), None);
    }

    #[test]
    fn parses_json_version() {
        assert_eq!(
            parse_json_version(r#"{"name":"nxtg-forge","version":"3.10.2"}"#).as_deref(),
            Some("3.10.2")
        );
        assert_eq!(parse_json_version(r#"{"name":"x"}"#), None);
        assert_eq!(parse_json_version("not json"), None);
    }
}
