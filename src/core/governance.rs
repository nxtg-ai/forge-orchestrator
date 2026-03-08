use crate::brain::{DriftScore, ForgeBrain};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Severity levels for governance findings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// A single governance finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub category: String,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Overall governance report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceReport {
    pub health_score: f32, // 0.0-100.0
    pub findings: Vec<Finding>,
    pub drift: Option<DriftReport>,
    pub summary: String,
}

/// Drift-specific sub-report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub vision_alignment: f32, // 0.0-1.0 (0 = aligned, 1 = drifted)
    pub explanation: String,
    pub completed_tasks: usize,
    pub total_tasks: usize,
}

/// Manages governance checks across the project
pub struct GovernanceChecker {
    project_root: PathBuf,
    forge_dir: PathBuf,
}

impl GovernanceChecker {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = project_root.into();
        let forge = root.join(".forge");
        Self {
            project_root: root,
            forge_dir: forge,
        }
    }

    /// Run all governance checks and produce a report
    pub fn full_check(&self, brain: &dyn ForgeBrain) -> GovernanceReport {
        let mut findings = Vec::new();

        // 1. Documentation checks
        findings.extend(self.check_documentation());

        // 2. Architecture checks
        findings.extend(self.check_architecture());

        // 3. Task health checks
        findings.extend(self.check_task_health());

        // 4. Knowledge coverage
        findings.extend(self.check_knowledge_coverage());

        // 5. Drift detection
        let drift = self.check_drift(brain);

        // Calculate health score
        let health_score = self.calculate_health_score(&findings, &drift);

        let critical_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        let warning_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();

        let summary = format!(
            "Health: {health_score:.0}/100 | {critical} critical, {warning} warnings, {info} info",
            critical = critical_count,
            warning = warning_count,
            info = findings.len() - critical_count - warning_count,
        );

        GovernanceReport {
            health_score,
            findings,
            drift,
            summary,
        }
    }

    /// Check documentation exists and is reasonably current
    fn check_documentation(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Check for SPEC.md
        let spec_path = self.project_root.join("SPEC.md");
        if !spec_path.exists() {
            findings.push(Finding {
                category: "documentation".into(),
                severity: Severity::Warning,
                message: "No SPEC.md found — project vision is undocumented".into(),
                suggestion: Some(
                    "Create SPEC.md with project vision, goals, and requirements".into(),
                ),
            });
        }

        // Check for README
        let readme_exists = self.project_root.join("README.md").exists()
            || self.project_root.join("readme.md").exists()
            || self.project_root.join("README").exists();
        if !readme_exists {
            findings.push(Finding {
                category: "documentation".into(),
                severity: Severity::Warning,
                message: "No README found".into(),
                suggestion: Some(
                    "Create README.md with project description and setup instructions".into(),
                ),
            });
        }

        // Check for plan
        let plan_path = self.forge_dir.join("plan.md");
        if !plan_path.exists() {
            findings.push(Finding {
                category: "documentation".into(),
                severity: Severity::Info,
                message: "No master plan — run `forge plan --generate` to create one".into(),
                suggestion: None,
            });
        }

        // Check plan staleness: if plan exists but all tasks are done, it may need refreshing
        if plan_path.exists() {
            let task_mgr = crate::core::task::TaskManager::new(&self.forge_dir);
            if let Ok(tasks) = task_mgr.list_tasks() {
                let all_done = !tasks.is_empty()
                    && tasks
                        .iter()
                        .all(|t| t.status == crate::core::task::TaskStatus::Completed);
                if all_done {
                    findings.push(Finding {
                        category: "documentation".into(),
                        severity: Severity::Info,
                        message: "All tasks completed — plan may need updating for next phase"
                            .into(),
                        suggestion: Some(
                            "Update SPEC.md and run `forge plan --generate` for the next phase"
                                .into(),
                        ),
                    });
                }
            }
        }

        findings
    }

    /// Check project structure and architecture conventions
    fn check_architecture(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Check .forge/ directory structure
        if !self.forge_dir.exists() {
            findings.push(Finding {
                category: "architecture".into(),
                severity: Severity::Critical,
                message: "No .forge/ directory — project is not initialized".into(),
                suggestion: Some("Run `forge init` to initialize the project".into()),
            });
            return findings;
        }

        // Check state.json
        let state_path = self.forge_dir.join("state.json");
        if !state_path.exists() {
            findings.push(Finding {
                category: "architecture".into(),
                severity: Severity::Critical,
                message: "Missing state.json — orchestration state is lost".into(),
                suggestion: Some("Run `forge init` to recreate state".into()),
            });
        }

        // Check events.jsonl
        let events_path = self.forge_dir.join("events.jsonl");
        if !events_path.exists() {
            findings.push(Finding {
                category: "architecture".into(),
                severity: Severity::Warning,
                message: "Missing events.jsonl — no event history".into(),
                suggestion: None,
            });
        }

        // Check tasks directory
        let tasks_dir = self.forge_dir.join("tasks");
        if !tasks_dir.exists()
            || tasks_dir
                .read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        {
            findings.push(Finding {
                category: "architecture".into(),
                severity: Severity::Info,
                message: "No tasks defined — generate a plan to create tasks".into(),
                suggestion: Some("Run `forge plan --generate` to create tasks from SPEC.md".into()),
            });
        }

        // Check for orphaned lock files
        let state_mgr = crate::core::state::StateManager::new(&self.forge_dir);
        if let Ok(state) = state_mgr.load()
            && !state.active_locks.is_empty()
        {
            let task_mgr = crate::core::task::TaskManager::new(&self.forge_dir);
            for task_id in state.active_locks.keys() {
                if let Ok(task) = task_mgr.get_task(task_id)
                    && (task.status == crate::core::task::TaskStatus::Completed
                        || task.status == crate::core::task::TaskStatus::Failed)
                {
                    findings.push(Finding {
                        category: "architecture".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "Orphaned file lock for {task_id} (task is {:?})",
                            task.status
                        ),
                        suggestion: Some("Run `forge sync` to clean up stale locks".into()),
                    });
                }
            }
        }

        findings
    }

    /// Check task health — stale tasks, unbalanced assignments, etc.
    fn check_task_health(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        let task_mgr = crate::core::task::TaskManager::new(&self.forge_dir);
        let tasks = match task_mgr.list_tasks() {
            Ok(t) => t,
            Err(_) => return findings,
        };

        if tasks.is_empty() {
            return findings;
        }

        // Check for stale in-progress tasks (assigned but no recent events)
        let now = chrono::Utc::now();
        for task in &tasks {
            if task.status == crate::core::task::TaskStatus::Assigned
                || task.status == crate::core::task::TaskStatus::InProgress
            {
                let hours_since_update = (now - task.updated_at).num_hours();
                if hours_since_update > 24 {
                    findings.push(Finding {
                        category: "task_health".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "Task {} has been in {:?} state for {}h",
                            task.id, task.status, hours_since_update
                        ),
                        suggestion: Some(format!(
                            "Check if {} is stuck — consider reassigning or marking as failed",
                            task.id
                        )),
                    });
                }
            }
        }

        // Check for tasks with unresolvable dependencies
        let completed_ids: Vec<String> = tasks
            .iter()
            .filter(|t| t.status == crate::core::task::TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();
        let all_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();

        for task in &tasks {
            if task.status == crate::core::task::TaskStatus::Pending {
                for dep in &task.depends_on {
                    if !all_ids.contains(dep) {
                        findings.push(Finding {
                            category: "task_health".into(),
                            severity: Severity::Critical,
                            message: format!(
                                "Task {} depends on {} which doesn't exist",
                                task.id, dep
                            ),
                            suggestion: Some(format!(
                                "Remove dependency on {dep} or create the missing task"
                            )),
                        });
                    }
                }
            }
        }

        // Check failed tasks
        let failed_count = tasks
            .iter()
            .filter(|t| t.status == crate::core::task::TaskStatus::Failed)
            .count();
        if failed_count > 0 {
            findings.push(Finding {
                category: "task_health".into(),
                severity: Severity::Warning,
                message: format!("{failed_count} task(s) in failed state"),
                suggestion: Some("Review failed tasks and either retry or remove them".into()),
            });
        }

        // Progress metric
        let completed_count = completed_ids.len();
        let total = tasks.len();
        let progress = (completed_count as f32 / total as f32) * 100.0;
        findings.push(Finding {
            category: "task_health".into(),
            severity: Severity::Info,
            message: format!(
                "Progress: {completed_count}/{total} tasks completed ({progress:.0}%)"
            ),
            suggestion: None,
        });

        findings
    }

    /// Check knowledge base coverage
    fn check_knowledge_coverage(&self) -> Vec<Finding> {
        let mut findings = Vec::new();

        let knowledge_mgr = crate::core::knowledge::KnowledgeManager::new(&self.forge_dir);
        let entries = knowledge_mgr.list(None).unwrap_or_default();

        if entries.is_empty() {
            findings.push(Finding {
                category: "knowledge".into(),
                severity: Severity::Info,
                message: "No knowledge captured yet — use forge_capture_knowledge to build the knowledge base".into(),
                suggestion: None,
            });
            return findings;
        }

        // Check category distribution
        let categories = ["research", "decisions", "learnings", "patterns"];
        let mut empty_categories = Vec::new();
        for cat in &categories {
            let count = entries.iter().filter(|e| e.category == *cat).count();
            if count == 0 {
                empty_categories.push(*cat);
            }
        }

        if !empty_categories.is_empty() {
            findings.push(Finding {
                category: "knowledge".into(),
                severity: Severity::Info,
                message: format!(
                    "No entries in: {} — consider capturing knowledge in these areas",
                    empty_categories.join(", ")
                ),
                suggestion: None,
            });
        }

        // Check if SKILL.md files are up to date
        let knowledge_dir = self.forge_dir.join("knowledge");
        for cat in &categories {
            let cat_entries: Vec<_> = entries.iter().filter(|e| e.category == *cat).collect();
            if !cat_entries.is_empty() {
                let skill_path = knowledge_dir.join(format!("{cat}-SKILL.md"));
                if !skill_path.exists() {
                    findings.push(Finding {
                        category: "knowledge".into(),
                        severity: Severity::Info,
                        message: format!(
                            "{} has {} entries but no SKILL.md — generate with forge_get_knowledge",
                            cat,
                            cat_entries.len()
                        ),
                        suggestion: Some(
                            "Call forge_get_knowledge with generate_skills: true".into(),
                        ),
                    });
                }
            }
        }

        findings.push(Finding {
            category: "knowledge".into(),
            severity: Severity::Info,
            message: format!(
                "Knowledge base: {} entries across {} categories",
                entries.len(),
                entries
                    .iter()
                    .map(|e| e.category.as_str())
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            ),
            suggestion: None,
        });

        findings
    }

    /// Run drift detection against the project vision
    fn check_drift(&self, brain: &dyn ForgeBrain) -> Option<DriftReport> {
        // Read the spec/vision
        let spec_path = self.project_root.join("SPEC.md");
        let vision = if spec_path.exists() {
            std::fs::read_to_string(&spec_path).ok()
        } else {
            // Try plan.md as fallback
            let plan_path = self.forge_dir.join("plan.md");
            if plan_path.exists() {
                std::fs::read_to_string(&plan_path).ok()
            } else {
                None
            }
        };

        let vision = vision?;

        // Build work summary from completed tasks + events
        let task_mgr = crate::core::task::TaskManager::new(&self.forge_dir);
        let tasks = task_mgr.list_tasks().unwrap_or_default();
        let total = tasks.len();
        let completed: Vec<_> = tasks
            .iter()
            .filter(|t| t.status == crate::core::task::TaskStatus::Completed)
            .collect();

        let work_summary = if completed.is_empty() {
            "No tasks completed yet.".to_string()
        } else {
            let task_summaries: Vec<String> = completed
                .iter()
                .map(|t| format!("- {}: {}", t.id, t.title))
                .collect();
            format!(
                "Completed tasks ({}/{}):\n{}",
                completed.len(),
                total,
                task_summaries.join("\n")
            )
        };

        // Use the brain for drift evaluation
        let drift_score = brain
            .evaluate_drift(&work_summary, &vision)
            .unwrap_or(DriftScore {
                score: 0.0,
                explanation: "Unable to evaluate drift".into(),
            });

        Some(DriftReport {
            vision_alignment: drift_score.score,
            explanation: drift_score.explanation,
            completed_tasks: completed.len(),
            total_tasks: total,
        })
    }

    /// Calculate an overall health score from findings and drift
    fn calculate_health_score(&self, findings: &[Finding], drift: &Option<DriftReport>) -> f32 {
        let mut score: f32 = 100.0;

        // Deduct for findings
        for finding in findings {
            match finding.severity {
                Severity::Critical => score -= 20.0,
                Severity::Warning => score -= 5.0,
                Severity::Info => {} // info doesn't reduce score
            }
        }

        // Deduct for drift
        if let Some(drift) = drift {
            // drift.vision_alignment is 0.0-1.0 where higher = more drift
            score -= drift.vision_alignment * 30.0;
        }

        score.clamp(0.0, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    fn setup_initialized_project(dir: &TempDir) {
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();
        std::fs::create_dir_all(forge_dir.join("knowledge")).unwrap();

        // Write state.json
        let state = crate::core::state::ForgeState {
            project_name: "TestProject".into(),
            ..Default::default()
        };
        let state_json = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(forge_dir.join("state.json"), state_json).unwrap();

        // Write events.jsonl
        std::fs::write(forge_dir.join("events.jsonl"), "").unwrap();

        // Write plan.md
        std::fs::write(forge_dir.join("plan.md"), "# Test Plan\n").unwrap();
    }

    /// Build a Finding with the given severity; category/message are
    /// irrelevant for score tests so we use fixed placeholder values.
    fn finding(severity: Severity) -> Finding {
        Finding {
            category: "test".into(),
            severity,
            message: "test finding".into(),
            suggestion: None,
        }
    }

    /// Build a DriftReport with the given vision_alignment score.
    fn drift_report(vision_alignment: f32) -> DriftReport {
        DriftReport {
            vision_alignment,
            explanation: "test drift".into(),
            completed_tasks: 0,
            total_tasks: 0,
        }
    }

    /// Write a Task JSON into the temp project so TaskManager can read it.
    fn write_task_json(dir: &TempDir, task: &crate::core::task::Task) {
        let forge_dir = dir.path().join(".forge");
        task.write_json(&forge_dir).unwrap();
    }

    /// Create a task with a specific status and updated_at timestamp.
    fn task_with_status_and_time(
        id: &str,
        status: crate::core::task::TaskStatus,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> crate::core::task::Task {
        let mut task = crate::core::task::Task::new(id, "Test task", "Description");
        task.status = status;
        task.updated_at = updated_at;
        task
    }

    // ---------------------------------------------------------------
    // A. calculate_health_score — exact value assertions
    // ---------------------------------------------------------------

    #[test]
    fn test_health_score_no_findings_no_drift() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        let score = checker.calculate_health_score(&[], &None);
        assert_eq!(score, 100.0);
    }

    #[test]
    fn test_health_score_one_critical() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        let findings = vec![finding(Severity::Critical)];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 80.0); // 100 - 20
    }

    #[test]
    fn test_health_score_one_warning() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        let findings = vec![finding(Severity::Warning)];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 95.0); // 100 - 5
    }

    #[test]
    fn test_health_score_info_no_deduction() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        let findings = vec![finding(Severity::Info)];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 100.0); // Info does NOT deduct
    }

    #[test]
    fn test_health_score_mixed_findings() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 2 critical (-40) + 3 warnings (-15) + 1 info (0) = 100 - 55 = 45
        let findings = vec![
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Warning),
            finding(Severity::Warning),
            finding(Severity::Warning),
            finding(Severity::Info),
        ];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 45.0);
    }

    #[test]
    fn test_health_score_with_drift_only() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // drift 0.5 → 100 - (0.5 * 30) = 85
        let drift = Some(drift_report(0.5));
        let score = checker.calculate_health_score(&[], &drift);
        assert_eq!(score, 85.0);
    }

    #[test]
    fn test_health_score_with_max_drift() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // drift 1.0 → 100 - 30 = 70
        let drift = Some(drift_report(1.0));
        let score = checker.calculate_health_score(&[], &drift);
        assert_eq!(score, 70.0);
    }

    #[test]
    fn test_health_score_with_zero_drift() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // drift 0.0 → 100 - 0 = 100
        let drift = Some(drift_report(0.0));
        let score = checker.calculate_health_score(&[], &drift);
        assert_eq!(score, 100.0);
    }

    #[test]
    fn test_health_score_findings_plus_drift() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 1 critical (-20) + drift 0.5 (-15) = 100 - 35 = 65
        let findings = vec![finding(Severity::Critical)];
        let drift = Some(drift_report(0.5));
        let score = checker.calculate_health_score(&findings, &drift);
        assert_eq!(score, 65.0);
    }

    #[test]
    fn test_health_score_clamps_to_zero() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 6 critical = 100 - 120 → clamped to 0.0
        let findings = vec![
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
        ];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_health_score_boundary_exactly_zero() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 5 critical = 100 - 100 = exactly 0.0
        let findings = vec![
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
            finding(Severity::Critical),
        ];
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_health_score_does_not_exceed_100() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // Empty findings, no drift → exactly 100, never above
        let score = checker.calculate_health_score(&[], &None);
        assert_eq!(score, 100.0);
        assert!(score <= 100.0);
    }

    #[test]
    fn test_health_score_twenty_warnings() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 20 warnings = 100 - 100 = exactly 0.0
        let findings: Vec<Finding> = (0..20).map(|_| finding(Severity::Warning)).collect();
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_health_score_twenty_one_warnings_clamps() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        // 21 warnings = 100 - 105 → clamped to 0.0
        let findings: Vec<Finding> = (0..21).map(|_| finding(Severity::Warning)).collect();
        let score = checker.calculate_health_score(&findings, &None);
        assert_eq!(score, 0.0);
    }

    // ---------------------------------------------------------------
    // B. check_documentation — count and severity assertions
    // ---------------------------------------------------------------

    #[test]
    fn test_check_documentation_no_spec_no_readme() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Should produce exactly 2 Warning findings (SPEC.md + README)
        let doc_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.category == "documentation" && f.severity == Severity::Warning)
            .collect();
        assert_eq!(doc_warnings.len(), 2);
        assert!(doc_warnings.iter().any(|f| f.message.contains("SPEC.md")));
        assert!(doc_warnings.iter().any(|f| f.message.contains("README")));
    }

    #[test]
    fn test_check_documentation_with_spec_and_readme() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Project").unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Zero documentation warnings
        let doc_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.category == "documentation" && f.severity == Severity::Warning)
            .collect();
        assert_eq!(doc_warnings.len(), 0);
    }

    #[test]
    fn test_check_documentation_missing_readme() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Should warn about missing README with Warning severity
        let readme_findings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.message.contains("README") && f.severity == Severity::Warning)
            .collect();
        assert_eq!(readme_findings.len(), 1);
        assert_eq!(readme_findings[0].category, "documentation");
    }

    #[test]
    fn test_check_documentation_no_plan_info_finding() {
        let dir = TempDir::new().unwrap();
        // Set up project WITHOUT plan.md
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();
        std::fs::create_dir_all(forge_dir.join("knowledge")).unwrap();
        let state = crate::core::state::ForgeState {
            project_name: "TestProject".into(),
            ..Default::default()
        };
        std::fs::write(
            forge_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();
        std::fs::write(forge_dir.join("events.jsonl"), "").unwrap();
        // No plan.md written

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Should have Info finding about missing plan
        let plan_info: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.message.contains("No master plan") && f.severity == Severity::Info)
            .collect();
        assert_eq!(plan_info.len(), 1);
    }

    #[test]
    fn test_check_documentation_all_tasks_completed_plan_stale() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Project").unwrap();

        // Create a completed task on disk
        let mut task = crate::core::task::Task::new("T-001", "Task 1", "Do something");
        task.status = crate::core::task::TaskStatus::Completed;
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Should have Info finding about plan needing update
        let plan_stale: Vec<&Finding> = findings
            .iter()
            .filter(|f| {
                f.message.contains("plan may need updating") && f.severity == Severity::Info
            })
            .collect();
        assert_eq!(plan_stale.len(), 1);
        assert_eq!(plan_stale[0].category, "documentation");
    }

    // ---------------------------------------------------------------
    // C. check_architecture — count, severity, early return
    // ---------------------------------------------------------------

    #[test]
    fn test_check_architecture_not_initialized() {
        let dir = TempDir::new().unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // No .forge/ → exactly 1 Critical finding, early return
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert!(findings[0].message.contains(".forge/"));
        assert_eq!(findings[0].category, "architecture");
    }

    #[test]
    fn test_check_architecture_no_state_json() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        // .forge/ exists but no state.json

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should have Critical for missing state.json
        let state_critical: Vec<&Finding> = findings
            .iter()
            .filter(|f| {
                f.severity == Severity::Critical && f.message.contains("state.json")
            })
            .collect();
        assert_eq!(state_critical.len(), 1);
    }

    #[test]
    fn test_check_architecture_no_events_jsonl() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();
        // Write state.json but NO events.jsonl
        let state = crate::core::state::ForgeState {
            project_name: "TestProject".into(),
            ..Default::default()
        };
        std::fs::write(
            forge_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should have Warning for missing events.jsonl
        let events_warning: Vec<&Finding> = findings
            .iter()
            .filter(|f| {
                f.severity == Severity::Warning && f.message.contains("events.jsonl")
            })
            .collect();
        assert_eq!(events_warning.len(), 1);
        assert_eq!(events_warning[0].category, "architecture");
    }

    #[test]
    fn test_check_architecture_no_tasks_dir_info() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        // state.json + events.jsonl but no tasks/ directory
        let state = crate::core::state::ForgeState {
            project_name: "TestProject".into(),
            ..Default::default()
        };
        std::fs::write(
            forge_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();
        std::fs::write(forge_dir.join("events.jsonl"), "").unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should have Info about no tasks
        let task_info: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Info && f.message.contains("No tasks defined"))
            .collect();
        assert_eq!(task_info.len(), 1);
    }

    #[test]
    fn test_check_architecture_initialized() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should have zero Critical findings
        let critical_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        assert_eq!(critical_count, 0);

        // Should have Info about empty tasks (tasks/ exists but is empty)
        let task_info_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Info && f.message.contains("No tasks defined"))
            .count();
        assert_eq!(task_info_count, 1);
    }

    #[test]
    fn test_check_architecture_early_return_on_missing_forge_dir() {
        let dir = TempDir::new().unwrap();
        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Early return: exactly 1 finding, no state.json check, no events check
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("not initialized"));
    }

    // ---------------------------------------------------------------
    // D. check_task_health — stale threshold, failed count, progress
    // ---------------------------------------------------------------

    #[test]
    fn test_task_health_empty_tasks() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // No tasks → no findings at all (returns early)
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_task_health_stale_task_over_24h() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Create task updated 25 hours ago, status InProgress
        let updated_at = chrono::Utc::now() - chrono::Duration::hours(25);
        let task = task_with_status_and_time(
            "T-001",
            crate::core::task::TaskStatus::InProgress,
            updated_at,
        );
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // Should have a Warning about stale task
        let stale_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| {
                f.severity == Severity::Warning
                    && f.category == "task_health"
                    && f.message.contains("T-001")
            })
            .collect();
        assert_eq!(stale_warnings.len(), 1);
        // Message should mention the hours (25)
        assert!(stale_warnings[0].message.contains("25h"));
    }

    #[test]
    fn test_task_health_not_stale_under_24h() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Create task updated 23 hours ago — should NOT be flagged
        let updated_at = chrono::Utc::now() - chrono::Duration::hours(23);
        let task = task_with_status_and_time(
            "T-001",
            crate::core::task::TaskStatus::InProgress,
            updated_at,
        );
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // No stale warnings
        let stale_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning && f.message.contains("T-001"))
            .collect();
        assert_eq!(stale_warnings.len(), 0);
    }

    #[test]
    fn test_task_health_assigned_task_stale() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Assigned status also triggers stale check
        let updated_at = chrono::Utc::now() - chrono::Duration::hours(30);
        let task = task_with_status_and_time(
            "T-002",
            crate::core::task::TaskStatus::Assigned,
            updated_at,
        );
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        let stale_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning && f.message.contains("T-002"))
            .collect();
        assert_eq!(stale_warnings.len(), 1);
        assert!(stale_warnings[0].message.contains("30h"));
    }

    #[test]
    fn test_task_health_completed_task_not_stale() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Completed tasks should NOT trigger stale warning
        let updated_at = chrono::Utc::now() - chrono::Duration::hours(100);
        let task = task_with_status_and_time(
            "T-001",
            crate::core::task::TaskStatus::Completed,
            updated_at,
        );
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        let stale_warnings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning && f.message.contains("state for"))
            .collect();
        assert_eq!(stale_warnings.len(), 0);
    }

    #[test]
    fn test_task_health_failed_count_in_message() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Create 2 failed tasks
        let mut t1 = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        t1.status = crate::core::task::TaskStatus::Failed;
        let mut t2 = crate::core::task::Task::new("T-002", "Task 2", "Desc");
        t2.status = crate::core::task::TaskStatus::Failed;
        let mut t3 = crate::core::task::Task::new("T-003", "Task 3", "Desc");
        t3.status = crate::core::task::TaskStatus::Pending;
        write_task_json(&dir, &t1);
        write_task_json(&dir, &t2);
        write_task_json(&dir, &t3);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // Should have warning with exact count "2 task(s) in failed state"
        let failed_findings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning && f.message.contains("failed state"))
            .collect();
        assert_eq!(failed_findings.len(), 1);
        assert!(failed_findings[0].message.contains("2 task(s)"));
    }

    #[test]
    fn test_task_health_progress_percentage() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // 1 completed + 2 pending = 1/3 = 33%
        let mut t1 = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        t1.status = crate::core::task::TaskStatus::Completed;
        let t2 = crate::core::task::Task::new("T-002", "Task 2", "Desc");
        let t3 = crate::core::task::Task::new("T-003", "Task 3", "Desc");
        write_task_json(&dir, &t1);
        write_task_json(&dir, &t2);
        write_task_json(&dir, &t3);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // Should have progress Info finding with "1/3 tasks completed (33%)"
        let progress_findings: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Info && f.message.contains("Progress:"))
            .collect();
        assert_eq!(progress_findings.len(), 1);
        assert!(progress_findings[0].message.contains("1/3"));
        assert!(progress_findings[0].message.contains("33%"));
    }

    #[test]
    fn test_task_health_unresolvable_dependency() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Task depends on non-existent task
        let mut task = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        task.depends_on = vec!["T-999".into()];
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // Should have Critical finding about missing dependency
        let dep_findings: Vec<&Finding> = findings
            .iter()
            .filter(|f| {
                f.severity == Severity::Critical
                    && f.message.contains("T-999")
                    && f.message.contains("doesn't exist")
            })
            .collect();
        assert_eq!(dep_findings.len(), 1);
    }

    #[test]
    fn test_task_health_resolvable_dependency_no_finding() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // T-001 depends on T-002 which exists
        let mut t1 = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        t1.depends_on = vec!["T-002".into()];
        let t2 = crate::core::task::Task::new("T-002", "Task 2", "Desc");
        write_task_json(&dir, &t1);
        write_task_json(&dir, &t2);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_task_health();

        // Should NOT have Critical about missing dependency
        let dep_critical: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Critical && f.message.contains("doesn't exist"))
            .collect();
        assert_eq!(dep_critical.len(), 0);
    }

    // ---------------------------------------------------------------
    // E. check_knowledge_coverage — content and count assertions
    // ---------------------------------------------------------------

    #[test]
    fn test_knowledge_coverage_empty() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_knowledge_coverage();

        // Empty knowledge → exactly 1 finding with specific message
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("No knowledge captured"));
        assert_eq!(findings[0].severity, Severity::Info);
        assert_eq!(findings[0].category, "knowledge");
    }

    #[test]
    fn test_knowledge_coverage_with_entries_missing_categories() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Add a "research" knowledge entry
        let entry = crate::core::knowledge::KnowledgeEntry::new(
            "Test finding",
            "Some research content",
            &crate::brain::KnowledgeCategory::Research,
        );
        let knowledge_mgr =
            crate::core::knowledge::KnowledgeManager::new(dir.path().join(".forge"));
        knowledge_mgr.capture(&entry).unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_knowledge_coverage();

        // Should NOT have "No knowledge captured"
        assert!(!findings.iter().any(|f| f.message.contains("No knowledge captured")));

        // Should have Info about missing categories (decisions, learnings, patterns)
        let missing_cat: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.message.contains("No entries in:"))
            .collect();
        assert_eq!(missing_cat.len(), 1);
        assert!(missing_cat[0].message.contains("decisions"));
        assert!(missing_cat[0].message.contains("learnings"));
        assert!(missing_cat[0].message.contains("patterns"));
        // "research" should NOT be listed as missing
        assert!(!missing_cat[0].message.contains("research"));
    }

    #[test]
    fn test_knowledge_coverage_entry_count_in_summary() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Add 2 research entries
        let knowledge_mgr =
            crate::core::knowledge::KnowledgeManager::new(dir.path().join(".forge"));
        let e1 = crate::core::knowledge::KnowledgeEntry::new(
            "Finding 1",
            "Content 1",
            &crate::brain::KnowledgeCategory::Research,
        );
        let e2 = crate::core::knowledge::KnowledgeEntry::new(
            "Finding 2",
            "Content 2",
            &crate::brain::KnowledgeCategory::Research,
        );
        knowledge_mgr.capture(&e1).unwrap();
        knowledge_mgr.capture(&e2).unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_knowledge_coverage();

        // Should have summary with "2 entries across 1 categories"
        let summary: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.message.contains("Knowledge base:"))
            .collect();
        assert_eq!(summary.len(), 1);
        assert!(summary[0].message.contains("2 entries"));
        assert!(summary[0].message.contains("1 categories"));
    }

    #[test]
    fn test_knowledge_coverage_missing_skill_md() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Add a research entry (creates knowledge/research/ dir with JSON)
        let knowledge_mgr =
            crate::core::knowledge::KnowledgeManager::new(dir.path().join(".forge"));
        let entry = crate::core::knowledge::KnowledgeEntry::new(
            "Finding",
            "Content",
            &crate::brain::KnowledgeCategory::Research,
        );
        knowledge_mgr.capture(&entry).unwrap();
        // No research-SKILL.md file

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_knowledge_coverage();

        // Should have Info about missing SKILL.md for research
        let skill_info: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.message.contains("SKILL.md") && f.message.contains("research"))
            .collect();
        assert_eq!(skill_info.len(), 1);
        assert_eq!(skill_info[0].severity, Severity::Info);
        // Should mention the count of entries
        assert!(skill_info[0].message.contains("1 entries"));
    }

    // ---------------------------------------------------------------
    // F. check_drift — value assertions
    // ---------------------------------------------------------------

    #[test]
    fn test_drift_without_vision() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let drift = checker.check_drift(&brain);

        // plan.md exists → drift is Some, RuleBasedBrain returns score 0.0
        assert!(drift.is_some());
        let drift = drift.unwrap();
        assert_eq!(drift.vision_alignment, 0.0);
        assert_eq!(drift.completed_tasks, 0);
        assert_eq!(drift.total_tasks, 0);
    }

    #[test]
    fn test_drift_no_spec_no_plan() {
        let dir = TempDir::new().unwrap();
        // Only create .forge/ without plan.md or SPEC.md
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let drift = checker.check_drift(&brain);

        // No vision document → drift is None
        assert!(drift.is_none());
    }

    #[test]
    fn test_drift_with_spec_and_completed_tasks() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision\nBuild a tool").unwrap();

        // Add one completed task
        let mut task = crate::core::task::Task::new("T-001", "Build core", "Build the core");
        task.status = crate::core::task::TaskStatus::Completed;
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let drift = checker.check_drift(&brain);

        let drift = drift.unwrap();
        assert_eq!(drift.completed_tasks, 1);
        assert_eq!(drift.total_tasks, 1);
        // RuleBasedBrain always returns 0.0 drift
        assert_eq!(drift.vision_alignment, 0.0);
    }

    #[test]
    fn test_drift_task_counts_accuracy() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision").unwrap();

        // 2 completed + 1 pending = total 3, completed 2
        let mut t1 = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        t1.status = crate::core::task::TaskStatus::Completed;
        let mut t2 = crate::core::task::Task::new("T-002", "Task 2", "Desc");
        t2.status = crate::core::task::TaskStatus::Completed;
        let t3 = crate::core::task::Task::new("T-003", "Task 3", "Desc");
        write_task_json(&dir, &t1);
        write_task_json(&dir, &t2);
        write_task_json(&dir, &t3);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let drift = checker.check_drift(&brain).unwrap();

        assert_eq!(drift.completed_tasks, 2);
        assert_eq!(drift.total_tasks, 3);
    }

    // ---------------------------------------------------------------
    // G. full_check integration — exact health_score values
    // ---------------------------------------------------------------

    #[test]
    fn test_full_check_no_spec() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Should have warning about missing SPEC.md
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("SPEC.md"))
        );
        // Missing SPEC.md (Warning -5) + missing README (Warning -5) + empty tasks Info +
        // empty knowledge Info = 100 - 10 = 90
        // (plan.md exists so no "no plan" info, drift returns 0.0 via RuleBasedBrain)
        assert_eq!(report.health_score, 90.0);
    }

    #[test]
    fn test_full_check_with_spec() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Create SPEC.md + README.md → eliminates 2 warnings
        std::fs::write(dir.path().join("SPEC.md"), "# Vision\nBuild something").unwrap();
        std::fs::write(dir.path().join("README.md"), "# TestProject").unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // No documentation Warnings
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.category == "documentation" && f.severity == Severity::Warning)
        );

        // Drift is Some (SPEC.md exists) with score 0.0 (RuleBasedBrain)
        assert!(report.drift.is_some());
        assert_eq!(report.drift.as_ref().unwrap().vision_alignment, 0.0);

        // Only Info findings remain (empty tasks, empty knowledge) → score = 100
        assert_eq!(report.health_score, 100.0);
    }

    #[test]
    fn test_full_check_uninitialized_project() {
        let dir = TempDir::new().unwrap();
        // Completely empty directory — no .forge/
        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Architecture: 1 Critical (no .forge/) → -20
        // Documentation: 2 Warnings (SPEC.md + README) + 1 Info (no plan) → -10
        // Knowledge: 1 Info (no knowledge) → 0
        // No drift (no vision doc)
        // Total: 100 - 20 - 10 = 70
        assert_eq!(report.health_score, 70.0);

        // Exactly 1 critical finding
        let critical_count = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        assert_eq!(critical_count, 1);

        // Exactly 2 warning findings
        let warning_count = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();
        assert_eq!(warning_count, 2);
    }

    #[test]
    fn test_health_score_critical_findings() {
        let dir = TempDir::new().unwrap();
        // Don't initialize — should get critical findings
        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Exact score: 70.0 (1 Critical + 2 Warnings)
        assert_eq!(report.health_score, 70.0);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.severity == Severity::Critical)
        );
    }

    #[test]
    fn test_full_check_summary_format() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Readme").unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Summary should contain the score and counts
        assert!(report.summary.contains("Health: 100/100"));
        assert!(report.summary.contains("0 critical"));
        assert!(report.summary.contains("0 warnings"));
    }

    #[test]
    fn test_full_check_with_failed_tasks_exact_score() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);
        std::fs::write(dir.path().join("SPEC.md"), "# Vision").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Readme").unwrap();

        // 1 failed task produces 1 Warning (-5)
        let mut task = crate::core::task::Task::new("T-001", "Task 1", "Desc");
        task.status = crate::core::task::TaskStatus::Failed;
        write_task_json(&dir, &task);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // 1 Warning from failed task → 100 - 5 = 95
        assert_eq!(report.health_score, 95.0);
    }
}
