//! Doctor aggregation — pure, no IO.
//!
//! Ports the aggregation shape of `asifctl`'s `doctor.rs`: sub-summaries are **injected** by the
//! caller, never gathered here. `cli::doctor` does the reading (files, git, governance checks)
//! and hands the results in. That keeps this module deterministic and testable with literals,
//! and it means a sub-check that cannot be resolved degrades to a visible `SKIP` instead of
//! silently vanishing from the report.
//!
//! Status vocabulary: `OK` · `WARN` · `FAIL` · `SKIP`.

use serde::{Deserialize, Serialize};

use super::governance::GovernanceReport;
use super::release_debt::ReleaseDebtReport;

/// Report schema identifier, so consumers can version-gate the JSON.
pub const DOCTOR_SCHEMA: &str = "forge.doctor.report.v1";

/// Health score below which the quality dimension is a hard failure.
pub const HEALTH_FAIL_THRESHOLD: f32 = 50.0;
/// Health score below which the quality dimension warns.
pub const HEALTH_WARN_THRESHOLD: f32 = 75.0;
/// Vision-alignment drift above which the drift dimension warns (0.0 = aligned, 1.0 = drifted).
pub const DRIFT_WARN_THRESHOLD: f32 = 0.3;
/// Vision-alignment drift above which the drift dimension fails.
pub const DRIFT_FAIL_THRESHOLD: f32 = 0.6;

/// One named dimension of the verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

/// The aggregate verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema: String,
    pub overall: String,
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
    pub skip: usize,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Fail-closed exit code: `FAIL` always exits non-zero; `--strict` escalates `WARN` too.
    ///
    /// `SKIP` never fails on its own — an unresolvable sub-check is reported, not fatal — but it
    /// is visible in the report so a green run cannot quietly mean "nothing ran".
    pub fn exit_code(&self, strict: bool) -> i32 {
        match self.overall.as_str() {
            "FAIL" => 1,
            "WARN" if strict => 1,
            _ => 0,
        }
    }
}

/// Everything the aggregation needs, already resolved by the caller.
///
/// Every field is optional: `None` means "could not be resolved" and produces a `SKIP`, which is
/// how the report stays honest about what it did not check.
#[derive(Debug, Clone, Default)]
pub struct DoctorOptions {
    /// Release-debt verdict from `core::release_debt::evaluate`.
    pub release_debt: Option<ReleaseDebtReport>,
    /// Governance health + drift from `core::governance::GovernanceChecker::full_check`.
    pub governance: Option<GovernanceReport>,
    /// Reason a sub-check was skipped, e.g. "not a git repository".
    pub skip_reason: Option<String>,
    /// Whether the configured brain can actually measure drift. False ⇒ drift reports `SKIP`
    /// rather than a neutral score that reads as a pass.
    pub drift_evaluable: bool,
}

fn check(name: &str, status: &str, detail: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status: status.to_string(),
        detail: detail.into(),
    }
}

/// `FAIL` beats `WARN` beats `OK`. Ported from `asifctl` `doctor.rs`.
pub fn overall_status(fail: usize, warn: usize) -> &'static str {
    if fail > 0 {
        "FAIL"
    } else if warn > 0 {
        "WARN"
    } else {
        "OK"
    }
}

fn release_debt_check(report: &ReleaseDebtReport) -> DoctorCheck {
    let detail = if report.findings.is_empty() {
        format!(
            "version={} tag={} commits_since_tag={}",
            report.version.as_deref().unwrap_or("unknown"),
            report.latest_tag.as_deref().unwrap_or("none"),
            report.commits_since_tag
        )
    } else {
        report
            .findings
            .iter()
            .map(|f| format!("{}: {}", f.kind, f.detail))
            .collect::<Vec<_>>()
            .join("; ")
    };
    check("release_debt", &report.status, detail)
}

fn quality_check(report: &GovernanceReport) -> DoctorCheck {
    let critical = report
        .findings
        .iter()
        .filter(|f| matches!(f.severity, super::governance::Severity::Critical))
        .count();

    // A critical finding outranks the numeric score. The 5-dimension score is an average, so a
    // single critical defect can sit under a comfortable 80/100 and pass unnoticed — precisely
    // the silent-pass this gate exists to stop.
    let status = if critical > 0 || report.health_score < HEALTH_FAIL_THRESHOLD {
        "FAIL"
    } else if report.health_score < HEALTH_WARN_THRESHOLD {
        "WARN"
    } else {
        "OK"
    };
    check(
        "quality",
        status,
        format!(
            "health={:.1} findings={} critical={}",
            report.health_score,
            report.findings.len(),
            critical
        ),
    )
}

fn drift_check(report: &GovernanceReport, evaluable: bool) -> DoctorCheck {
    if !evaluable {
        // The rule-based brain returns a neutral 0.0 that is indistinguishable from "genuinely
        // aligned". Reporting that as OK manufactures a green. SKIP states the truth: not checked.
        return check(
            "drift",
            "SKIP",
            "drift needs an LLM brain — the rule-based brain returns a neutral score, not a measurement",
        );
    }
    match &report.drift {
        Some(drift) => {
            let status = if drift.vision_alignment > DRIFT_FAIL_THRESHOLD {
                "FAIL"
            } else if drift.vision_alignment > DRIFT_WARN_THRESHOLD {
                "WARN"
            } else {
                "OK"
            };
            check(
                "drift",
                status,
                format!(
                    "vision_alignment={:.2} tasks={}/{} — {}",
                    drift.vision_alignment,
                    drift.completed_tasks,
                    drift.total_tasks,
                    drift.explanation
                ),
            )
        }
        None => check("drift", "SKIP", "no drift analysis available (no SPEC.md?)"),
    }
}

/// Aggregate the injected sub-summaries into one verdict.
pub fn run_doctor(options: &DoctorOptions) -> DoctorReport {
    let mut checks = Vec::new();

    // 1. Release debt — the gate this directive exists for.
    checks.push(match &options.release_debt {
        Some(report) => release_debt_check(report),
        None => check(
            "release_debt",
            "SKIP",
            options
                .skip_reason
                .clone()
                .unwrap_or_else(|| "release debt not evaluated".to_string()),
        ),
    });

    // 2 & 3. Quality health and drift both come from the governance report, but they are
    //        reported as separate dimensions because they fail for unrelated reasons.
    match &options.governance {
        Some(report) => {
            checks.push(quality_check(report));
            checks.push(drift_check(report, options.drift_evaluable));
        }
        None => {
            checks.push(check("quality", "SKIP", "governance check not run"));
            checks.push(check("drift", "SKIP", "governance check not run"));
        }
    }

    let ok = checks.iter().filter(|c| c.status == "OK").count();
    let warn = checks.iter().filter(|c| c.status == "WARN").count();
    let fail = checks.iter().filter(|c| c.status == "FAIL").count();
    let skip = checks.iter().filter(|c| c.status == "SKIP").count();

    DoctorReport {
        schema: DOCTOR_SCHEMA.to_string(),
        overall: overall_status(fail, warn).to_string(),
        ok,
        warn,
        fail,
        skip,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::governance::{DriftReport, Finding, Severity};
    use crate::core::release_debt::{DebtFinding, ReleaseDebtReport};

    fn debt(status: &str, findings: Vec<DebtFinding>) -> ReleaseDebtReport {
        ReleaseDebtReport {
            status: status.into(),
            version: Some("1.5.2".into()),
            latest_tag: Some("v1.5.2".into()),
            commits_since_tag: 0,
            findings,
        }
    }

    fn governance(health: f32, drift: Option<f32>) -> GovernanceReport {
        GovernanceReport {
            health_score: health,
            findings: Vec::new(),
            drift: drift.map(|alignment| DriftReport {
                vision_alignment: alignment,
                explanation: "test".into(),
                completed_tasks: 1,
                total_tasks: 2,
            }),
            summary: "test".into(),
        }
    }

    #[test]
    fn clean_repo_is_ok_and_exits_zero_even_strict() {
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(90.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(report.overall, "OK");
        assert_eq!(report.fail, 0);
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.exit_code(true), 0, "clean repo must pass --strict");
        assert_eq!(report.schema, DOCTOR_SCHEMA);
    }

    #[test]
    fn fail_exits_nonzero_regardless_of_strict() {
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt(
                "FAIL",
                vec![DebtFinding {
                    kind: "lockfile-desync".into(),
                    status: "FAIL".into(),
                    detail: "Cargo.lock declares 1.5.0 but manifest declares 1.5.1".into(),
                }],
            )),
            governance: Some(governance(90.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(report.overall, "FAIL");
        assert_eq!(
            report.exit_code(false),
            1,
            "FAIL is fail-closed without --strict"
        );
        assert_eq!(report.exit_code(true), 1);
    }

    #[test]
    fn warn_exits_zero_normally_and_one_under_strict() {
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt(
                "WARN",
                vec![DebtFinding {
                    kind: "unreleased-commits".into(),
                    status: "WARN".into(),
                    detail: "9 commits since v1.5.1".into(),
                }],
            )),
            governance: Some(governance(90.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(report.overall, "WARN");
        assert_eq!(report.exit_code(false), 0);
        assert_eq!(report.exit_code(true), 1, "--strict escalates WARN");
    }

    #[test]
    fn unresolvable_sub_checks_skip_visibly_rather_than_vanish() {
        let report = run_doctor(&DoctorOptions {
            release_debt: None,
            governance: None,
            skip_reason: Some("not a git repository".into()),
            drift_evaluable: true,
        });
        assert_eq!(report.skip, 3, "every dimension must still be reported");
        assert_eq!(report.checks.len(), 3);
        assert_eq!(report.overall, "OK", "SKIP alone is not a failure");
        assert!(
            report.checks[0].detail.contains("not a git repository"),
            "the skip reason must reach the report: {}",
            report.checks[0].detail
        );
    }

    #[test]
    fn quality_thresholds() {
        let fail = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(40.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(fail.overall, "FAIL");

        let warn = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(60.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(warn.overall, "WARN");
    }

    #[test]
    fn drift_thresholds_and_missing_drift_skips() {
        let drifted = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(90.0, Some(0.8))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(drifted.overall, "FAIL");

        let no_drift = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(90.0, None)),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(no_drift.overall, "OK");
        assert_eq!(no_drift.skip, 1);
    }

    #[test]
    fn worst_status_wins_across_dimensions() {
        // WARN on release debt, FAIL on quality: the aggregate must be FAIL.
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt(
                "WARN",
                vec![DebtFinding {
                    kind: "tag-drift".into(),
                    status: "WARN".into(),
                    detail: "manifest ahead of tag".into(),
                }],
            )),
            governance: Some(governance(10.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(report.overall, "FAIL");
        assert_eq!(report.warn, 1);
        assert_eq!(report.fail, 1);
    }

    #[test]
    fn findings_reach_the_detail_string() {
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt(
                "FAIL",
                vec![DebtFinding {
                    kind: "multi-surface-drift".into(),
                    status: "FAIL".into(),
                    detail: "package.json=3.7.0, plugin.json=3.10.2".into(),
                }],
            )),
            governance: Some(governance(90.0, Some(0.1))),
            skip_reason: None,
            drift_evaluable: true,
        });
        let debt_check = &report.checks[0];
        assert!(debt_check.detail.contains("3.7.0"), "{}", debt_check.detail);
        assert!(
            debt_check.detail.contains("multi-surface-drift"),
            "{}",
            debt_check.detail
        );
    }

    #[test]
    fn critical_findings_are_counted_in_quality_detail() {
        let mut gov = governance(90.0, Some(0.1));
        gov.findings.push(Finding {
            severity: Severity::Critical,
            category: "security".into(),
            message: "hardcoded secret".into(),
            suggestion: None,
        });
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(gov),
            skip_reason: None,
            drift_evaluable: true,
        });
        let quality = report
            .checks
            .iter()
            .find(|c| c.name == "quality")
            .expect("quality dimension");
        assert!(quality.detail.contains("critical=1"), "{}", quality.detail);
        assert_eq!(
            quality.status, "FAIL",
            "a critical finding must not pass on a comfortable average score"
        );
        assert_eq!(report.overall, "FAIL");
    }

    #[test]
    fn a_single_critical_finding_fails_despite_a_passing_score() {
        // Real case that motivated this rule: forge doctor reported health=80.0 with critical=1
        // as [ OK ], because the 5-dimension average absorbed the critical defect.
        let mut gov = governance(80.0, Some(0.1));
        gov.findings.push(Finding {
            severity: Severity::Critical,
            category: "architecture".into(),
            message: "Missing state.json".into(),
            suggestion: None,
        });
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(gov),
            skip_reason: None,
            drift_evaluable: true,
        });
        assert_eq!(report.overall, "FAIL");
        assert_eq!(report.exit_code(false), 1, "must fail without --strict too");
    }

    #[test]
    fn drift_skips_when_the_brain_cannot_measure_it() {
        // The rule-based brain returns a neutral 0.0 that is indistinguishable from "aligned".
        // Reporting that as OK would manufacture a green.
        let report = run_doctor(&DoctorOptions {
            release_debt: Some(debt("OK", vec![])),
            governance: Some(governance(90.0, Some(0.0))),
            skip_reason: None,
            drift_evaluable: false,
        });
        let drift = report
            .checks
            .iter()
            .find(|c| c.name == "drift")
            .expect("drift dimension");
        assert_eq!(drift.status, "SKIP");
        assert!(drift.detail.contains("LLM brain"), "{}", drift.detail);
        assert_eq!(report.overall, "OK", "an honest SKIP is not a failure");
    }

    #[test]
    fn overall_status_precedence() {
        assert_eq!(overall_status(1, 1), "FAIL");
        assert_eq!(overall_status(0, 1), "WARN");
        assert_eq!(overall_status(0, 0), "OK");
    }
}
