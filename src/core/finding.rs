use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Positive,
}

impl std::fmt::Display for FindingSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingSeverity::Critical => write!(f, "critical"),
            FindingSeverity::High => write!(f, "high"),
            FindingSeverity::Medium => write!(f, "medium"),
            FindingSeverity::Low => write!(f, "low"),
            FindingSeverity::Positive => write!(f, "positive"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FindingType {
    Bug,
    Missing,
    Enhancement,
    Positive,
}

impl std::fmt::Display for FindingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingType::Bug => write!(f, "bug"),
            FindingType::Missing => write!(f, "missing"),
            FindingType::Enhancement => write!(f, "enhancement"),
            FindingType::Positive => write!(f, "positive"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub description: String,
    pub severity: FindingSeverity,
    pub finding_type: FindingType,
    pub related_tasks: Vec<String>,
    pub created_at: DateTime<Utc>,
}

pub struct FindingManager {
    forge_dir: PathBuf,
}

impl FindingManager {
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    fn findings_dir(&self) -> PathBuf {
        self.forge_dir.join("findings")
    }

    pub fn save_finding(&self, finding: &Finding) -> anyhow::Result<()> {
        let dir = self.findings_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", finding.id));
        let content = serde_json::to_string_pretty(finding)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn list_findings(&self) -> anyhow::Result<Vec<Finding>> {
        let dir = self.findings_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut findings = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = std::fs::read_to_string(&path)?;
                if let Ok(finding) = serde_json::from_str::<Finding>(&content) {
                    findings.push(finding);
                }
            }
        }
        findings.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(findings)
    }

    pub fn next_finding_number(&self) -> anyhow::Result<u32> {
        let dir = self.findings_dir();
        if !dir.exists() {
            return Ok(1);
        }
        let mut max_id: u32 = 0;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name
                .strip_prefix("F-")
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(num) = num_str.parse::<u32>()
            {
                max_id = max_id.max(num);
            }
        }
        Ok(max_id + 1)
    }
}

/// Classify a finding description into severity and type using keyword heuristics.
pub fn classify_finding(description: &str) -> (FindingSeverity, FindingType) {
    let lower = description.to_lowercase();

    let severity =
        if lower.contains("crash") || lower.contains("data loss") || lower.contains("security") {
            FindingSeverity::Critical
        } else if lower.contains("broken")
            || lower.contains("doesn't work")
            || lower.contains("fail")
            || lower.contains("error")
        {
            FindingSeverity::High
        } else if lower.contains("slow")
            || lower.contains("confusing")
            || lower.contains("unclear")
            || lower.contains("should")
        {
            FindingSeverity::Medium
        } else if lower.contains("love")
            || lower.contains("great")
            || lower.contains("fast")
            || lower.contains("nice")
            || lower.contains("excellent")
        {
            FindingSeverity::Positive
        } else {
            FindingSeverity::Low
        };

    let finding_type = if matches!(severity, FindingSeverity::Positive) {
        FindingType::Positive
    } else if lower.contains("missing")
        || lower.contains("need")
        || lower.contains("should have")
        || lower.contains("add")
    {
        FindingType::Missing
    } else if lower.contains("improve") || lower.contains("better") || lower.contains("enhance") {
        FindingType::Enhancement
    } else {
        FindingType::Bug
    };

    (severity, finding_type)
}

/// Stop words that match too many tasks and produce noisy relations.
const RELATION_STOP_WORDS: &[&str] = &[
    "test",
    "testing",
    "that",
    "this",
    "with",
    "from",
    "have",
    "does",
    "should",
    "could",
    "would",
    "when",
    "what",
    "where",
    "which",
    "their",
    "there",
    "these",
    "those",
    "about",
    "after",
    "before",
    "between",
    "implement",
    "update",
    "create",
    "build",
    "make",
    "work",
    "working",
    "capture",
    "inline",
    "finding",
    "issue",
    "input",
    "output",
];

/// Find related tasks by matching keywords from the description against task titles.
/// Requires words with 5+ chars and filters common stop words to reduce false positives.
pub fn find_related_tasks(description: &str, tasks: &[crate::core::task::Task]) -> Vec<String> {
    let lower = description.to_lowercase();
    let keywords: Vec<&str> = lower
        .split_whitespace()
        .filter(|w| w.len() >= 5)
        .filter(|w| !RELATION_STOP_WORDS.contains(w))
        .collect();

    if keywords.is_empty() {
        return Vec::new();
    }

    tasks
        .iter()
        .filter(|t| {
            let title_lower = t.title.to_lowercase();
            keywords.iter().any(|w| title_lower.contains(w))
        })
        .take(5) // cap at 5 related tasks max
        .map(|t| t.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::Task;

    #[test]
    fn test_finding_serialization_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FindingManager::new(tmp.path());

        let finding = Finding {
            id: "F-001".to_string(),
            description: "Login page crashes on submit".to_string(),
            severity: FindingSeverity::Critical,
            finding_type: FindingType::Bug,
            related_tasks: vec!["T-003".to_string()],
            created_at: Utc::now(),
        };

        mgr.save_finding(&finding).unwrap();
        let loaded = mgr.list_findings().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "F-001");
        assert_eq!(loaded[0].severity, FindingSeverity::Critical);
        assert_eq!(loaded[0].finding_type, FindingType::Bug);
    }

    #[test]
    fn test_classify_finding_crash_critical() {
        let (severity, _) = classify_finding("The app crashes when I click submit");
        assert_eq!(severity, FindingSeverity::Critical);
    }

    #[test]
    fn test_classify_finding_broken_high() {
        let (severity, _) = classify_finding("The search is broken");
        assert_eq!(severity, FindingSeverity::High);
    }

    #[test]
    fn test_classify_finding_slow_medium() {
        let (severity, _) = classify_finding("Dashboard loads slow");
        assert_eq!(severity, FindingSeverity::Medium);
    }

    #[test]
    fn test_classify_finding_positive() {
        let (severity, finding_type) = classify_finding("I love the new status page");
        assert_eq!(severity, FindingSeverity::Positive);
        assert_eq!(finding_type, FindingType::Positive);
    }

    #[test]
    fn test_classify_finding_default_low() {
        let (severity, _) = classify_finding("The button is blue");
        assert_eq!(severity, FindingSeverity::Low);
    }

    #[test]
    fn test_classify_finding_type_missing() {
        let (_, finding_type) = classify_finding("Missing logout button, error when clicking");
        assert_eq!(finding_type, FindingType::Missing);
    }

    #[test]
    fn test_classify_finding_type_enhancement() {
        let (_, finding_type) = classify_finding("Should improve the loading speed");
        assert_eq!(finding_type, FindingType::Enhancement);
    }

    #[test]
    fn test_classify_finding_type_bug_default() {
        let (_, finding_type) = classify_finding("The form validation fails on submit");
        assert_eq!(finding_type, FindingType::Bug);
    }

    #[test]
    fn test_find_related_tasks_matches_keywords() {
        let tasks = vec![
            Task::new("T-001", "Auth module login", "desc"),
            Task::new("T-002", "Dashboard stats", "desc"),
            Task::new("T-003", "API endpoints", "desc"),
        ];
        let related = find_related_tasks("The login page is broken", &tasks);
        assert!(related.contains(&"T-001".to_string()));
    }

    #[test]
    fn test_find_related_tasks_no_match() {
        let tasks = vec![Task::new("T-001", "Auth module", "desc")];
        let related = find_related_tasks("xyz qqq", &tasks);
        assert!(related.is_empty());
    }

    #[test]
    fn test_next_finding_number_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FindingManager::new(tmp.path());
        assert_eq!(mgr.next_finding_number().unwrap(), 1);
    }

    #[test]
    fn test_next_finding_number_monotonic() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FindingManager::new(tmp.path());

        for i in [1, 3] {
            let finding = Finding {
                id: format!("F-{i:03}"),
                description: "test".to_string(),
                severity: FindingSeverity::Low,
                finding_type: FindingType::Bug,
                related_tasks: vec![],
                created_at: Utc::now(),
            };
            mgr.save_finding(&finding).unwrap();
        }
        assert_eq!(mgr.next_finding_number().unwrap(), 4);
    }

    #[test]
    fn test_finding_severity_display() {
        assert_eq!(FindingSeverity::Critical.to_string(), "critical");
        assert_eq!(FindingSeverity::High.to_string(), "high");
        assert_eq!(FindingSeverity::Medium.to_string(), "medium");
        assert_eq!(FindingSeverity::Low.to_string(), "low");
        assert_eq!(FindingSeverity::Positive.to_string(), "positive");
    }
}
