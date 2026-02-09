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
    }

    #[test]
    fn test_full_check_with_spec() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        // Create SPEC.md
        std::fs::write(dir.path().join("SPEC.md"), "# Vision\nBuild something").unwrap();
        // Create README.md
        std::fs::write(dir.path().join("README.md"), "# TestProject").unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Should NOT have documentation warnings about SPEC or README
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.category == "documentation" && f.severity == Severity::Warning)
        );

        // Should have drift report since SPEC.md exists
        assert!(report.drift.is_some());
    }

    #[test]
    fn test_health_score_critical_findings() {
        let dir = TempDir::new().unwrap();
        // Don't initialize — should get critical findings
        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let report = checker.full_check(&brain);

        // Score should be reduced for critical issues
        assert!(report.health_score < 100.0);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.severity == Severity::Critical)
        );
    }

    #[test]
    fn test_check_documentation_missing_readme() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_documentation();

        // Should warn about missing README
        assert!(findings.iter().any(|f| f.message.contains("README")));
    }

    #[test]
    fn test_check_architecture_initialized() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should NOT have critical architecture issues (project is initialized)
        assert!(!findings.iter().any(|f| f.severity == Severity::Critical));
    }

    #[test]
    fn test_check_architecture_not_initialized() {
        let dir = TempDir::new().unwrap();

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_architecture();

        // Should have critical: no .forge/
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Critical && f.message.contains(".forge/"))
        );
    }

    #[test]
    fn test_drift_without_vision() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let brain = crate::brain::rule_based::RuleBasedBrain;
        let drift = checker.check_drift(&brain);

        // plan.md exists, so drift should be Some (uses plan as fallback)
        assert!(drift.is_some());
    }

    #[test]
    fn test_knowledge_coverage_empty() {
        let dir = TempDir::new().unwrap();
        setup_initialized_project(&dir);

        let checker = GovernanceChecker::new(dir.path());
        let findings = checker.check_knowledge_coverage();

        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("No knowledge captured"))
        );
    }
}
