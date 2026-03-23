use crate::core::event::{EventLogger, EventType, ForgeEvent};
use crate::core::task::{Task, TaskManager, TaskStatus};
use chrono::Utc;
use std::path::{Path, PathBuf};

/// Result of the ship phase — summary of what was done.
#[derive(Debug)]
pub struct ShipResult {
    pub changelog: String,
    pub archived_tasks: usize,
    pub archived_to: Option<PathBuf>,
    pub version_suggestion: VersionBump,
    pub cleaned: bool,
}

/// Suggested version bump based on task types.
#[derive(Debug)]
pub enum VersionBump {
    Major,
    Minor,
    Patch,
    None,
}

impl std::fmt::Display for VersionBump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionBump::Major => write!(f, "major"),
            VersionBump::Minor => write!(f, "minor"),
            VersionBump::Patch => write!(f, "patch"),
            VersionBump::None => write!(f, "none"),
        }
    }
}

/// Generate a Keep-a-Changelog formatted entry from completed tasks.
pub fn generate_changelog(forge_dir: &Path) -> anyhow::Result<String> {
    let task_mgr = TaskManager::new(forge_dir);
    let tasks = task_mgr.list_tasks()?;

    let completed: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed && t.id.starts_with("T-"))
        .collect();

    if completed.is_empty() {
        return Ok("No completed tasks to include in changelog.".to_string());
    }

    let date = Utc::now().format("%Y-%m-%d");
    let mut added = Vec::new();
    let mut fixed = Vec::new();
    let mut changed = Vec::new();

    for task in &completed {
        let task_type = task.task_type.as_deref().unwrap_or("");
        let entry = format!("- {} ({})", task.title, task.id);
        match task_type {
            "implement" | "design" | "test" | "document" | "" => added.push(entry),
            "fix" => fixed.push(entry),
            _ => changed.push(entry),
        }
    }

    let mut changelog = format!("## [Unreleased] - {date}\n\n");

    if !added.is_empty() {
        changelog.push_str("### Added\n\n");
        for entry in &added {
            changelog.push_str(entry);
            changelog.push('\n');
        }
        changelog.push('\n');
    }

    if !fixed.is_empty() {
        changelog.push_str("### Fixed\n\n");
        for entry in &fixed {
            changelog.push_str(entry);
            changelog.push('\n');
        }
        changelog.push('\n');
    }

    if !changed.is_empty() {
        changelog.push_str("### Changed\n\n");
        for entry in &changed {
            changelog.push_str(entry);
            changelog.push('\n');
        }
        changelog.push('\n');
    }

    Ok(changelog)
}

/// Archive completed tasks, results, and signals to .forge/archive/{date}/.
pub fn archive_cycle(forge_dir: &Path) -> anyhow::Result<(usize, PathBuf)> {
    let timestamp = Utc::now().format("%Y-%m-%d-%H%M");
    let archive_dir = forge_dir.join("archive").join(timestamp.to_string());
    std::fs::create_dir_all(&archive_dir)?;

    let mut archived = 0;

    // Archive task files
    let tasks_dir = forge_dir.join("tasks");
    let archive_tasks = archive_dir.join("tasks");
    if tasks_dir.exists() {
        std::fs::create_dir_all(&archive_tasks)?;
        for entry in std::fs::read_dir(&tasks_dir)?.flatten() {
            let dest = archive_tasks.join(entry.file_name());
            std::fs::rename(entry.path(), dest)?;
            archived += 1;
        }
    }

    // Copy events.jsonl (keep original for historical queries)
    let events_src = forge_dir.join("events.jsonl");
    if events_src.exists() {
        std::fs::copy(&events_src, archive_dir.join("events.jsonl"))?;
    }

    // Archive results
    let results_dir = forge_dir.join("results");
    let archive_results = archive_dir.join("results");
    if results_dir.exists() {
        copy_dir_contents(&results_dir, &archive_results)?;
        // Clean results after archiving
        std::fs::remove_dir_all(&results_dir)?;
        std::fs::create_dir_all(&results_dir)?;
    }

    // Archive signals
    let signals_dir = forge_dir.join("signals");
    let archive_signals = archive_dir.join("signals");
    if signals_dir.exists() {
        copy_dir_contents(&signals_dir, &archive_signals)?;
        std::fs::remove_dir_all(&signals_dir)?;
        std::fs::create_dir_all(&signals_dir)?;
    }

    // Log archive event
    let event_logger = EventLogger::new(forge_dir);
    event_logger
        .log(&ForgeEvent::new(
            EventType::StateReconciled,
            format!("Archived {archived} artifacts to {}", archive_dir.display()),
        ))
        .ok();

    Ok((archived, archive_dir))
}

/// Suggest a version bump based on completed task types.
pub fn suggest_version_bump(forge_dir: &Path) -> VersionBump {
    let task_mgr = TaskManager::new(forge_dir);
    let tasks = match task_mgr.list_tasks() {
        Ok(t) => t,
        Err(_) => return VersionBump::None,
    };

    let completed: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed && t.id.starts_with("T-"))
        .collect();

    if completed.is_empty() {
        return VersionBump::None;
    }

    let has_features = completed.iter().any(|t| {
        matches!(
            t.task_type.as_deref(),
            Some("implement") | Some("design") | None
        )
    });
    let has_only_fixes = completed
        .iter()
        .all(|t| t.task_type.as_deref() == Some("fix"));

    if has_only_fixes {
        VersionBump::Patch
    } else if has_features {
        VersionBump::Minor
    } else {
        VersionBump::Patch
    }
}

/// Reset state.json and clean completed tasks for the next cycle.
pub fn clean_state(forge_dir: &Path) -> anyhow::Result<()> {
    use crate::core::state::StateManager;

    let state_mgr = StateManager::new(forge_dir);
    if let Ok(mut state) = state_mgr.load() {
        state.task_summary = Default::default();
        state.current_phase = None;
        state.active_locks.clear();
        state.updated_at = Utc::now();
        state_mgr.save(&state)?;
    }

    // Clear remaining task files (should already be archived)
    let tasks_dir = forge_dir.join("tasks");
    if tasks_dir.exists() {
        for entry in std::fs::read_dir(&tasks_dir)?.flatten() {
            std::fs::remove_file(entry.path()).ok();
        }
    }

    // Log cycle complete
    let event_logger = EventLogger::new(forge_dir);
    event_logger.log(&ForgeEvent::new(
        EventType::StateReconciled,
        "Ship phase complete — state cleaned for next cycle",
    ))?;

    Ok(())
}

/// Copy directory contents (non-recursive, files only).
fn copy_dir_contents(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), dest.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::{Task, TaskPhase};

    fn setup_forge_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(forge_dir.join("tasks")).unwrap();
        std::fs::create_dir_all(forge_dir.join("results")).unwrap();
        std::fs::create_dir_all(forge_dir.join("signals")).unwrap();
        (tmp, forge_dir)
    }

    fn create_completed_task(forge_dir: &Path, id: &str, title: &str, task_type: &str) {
        let mgr = TaskManager::new(forge_dir);
        let mut task = Task::new(id, title, "Description");
        task.status = TaskStatus::Completed;
        task.task_type = Some(task_type.to_string());
        task.phase = Some(TaskPhase::Build);
        mgr.create_task(&task).unwrap();
    }

    #[test]
    fn test_generate_changelog_empty() {
        let (_tmp, forge_dir) = setup_forge_dir();
        let cl = generate_changelog(&forge_dir).unwrap();
        assert!(cl.contains("No completed tasks"));
    }

    #[test]
    fn test_generate_changelog_with_tasks() {
        let (_tmp, forge_dir) = setup_forge_dir();
        create_completed_task(&forge_dir, "T-001", "Add high score", "implement");
        create_completed_task(&forge_dir, "T-002", "Fix combo bug", "fix");
        create_completed_task(&forge_dir, "T-003", "Add dark mode", "implement");

        let cl = generate_changelog(&forge_dir).unwrap();
        assert!(cl.contains("### Added"));
        assert!(cl.contains("Add high score (T-001)"));
        assert!(cl.contains("Add dark mode (T-003)"));
        assert!(cl.contains("### Fixed"));
        assert!(cl.contains("Fix combo bug (T-002)"));
    }

    #[test]
    fn test_generate_changelog_skips_non_t_tasks() {
        let (_tmp, forge_dir) = setup_forge_dir();
        create_completed_task(&forge_dir, "T-001", "Build feature", "implement");
        // V-xxx tasks should be excluded
        let mgr = TaskManager::new(&forge_dir);
        let mut verify = Task::new("V-001", "Verify feature", "desc");
        verify.status = TaskStatus::Completed;
        verify.phase = Some(TaskPhase::Verify);
        mgr.create_task(&verify).unwrap();

        let cl = generate_changelog(&forge_dir).unwrap();
        assert!(cl.contains("Build feature"));
        assert!(!cl.contains("Verify feature"));
    }

    #[test]
    fn test_archive_cycle() {
        let (_tmp, forge_dir) = setup_forge_dir();
        // Create some task files and results
        std::fs::write(forge_dir.join("tasks/T-001.json"), "{}").unwrap();
        std::fs::write(forge_dir.join("tasks/T-001.md"), "# T-001").unwrap();
        std::fs::write(forge_dir.join("results/T-001.txt"), "output").unwrap();
        std::fs::write(forge_dir.join("events.jsonl"), "{}\n").unwrap();

        let (count, archive_path) = archive_cycle(&forge_dir).unwrap();
        assert_eq!(count, 2); // T-001.json + T-001.md
        assert!(archive_path.exists());
        assert!(archive_path.join("tasks/T-001.json").exists());
        assert!(archive_path.join("results/T-001.txt").exists());
        assert!(archive_path.join("events.jsonl").exists());
        // Original tasks should be moved
        assert!(!forge_dir.join("tasks/T-001.json").exists());
        // Results should be cleaned
        assert!(!forge_dir.join("results/T-001.txt").exists());
    }

    #[test]
    fn test_suggest_version_bump_features() {
        let (_tmp, forge_dir) = setup_forge_dir();
        create_completed_task(&forge_dir, "T-001", "New feature", "implement");
        assert!(matches!(
            suggest_version_bump(&forge_dir),
            VersionBump::Minor
        ));
    }

    #[test]
    fn test_suggest_version_bump_fixes_only() {
        let (_tmp, forge_dir) = setup_forge_dir();
        create_completed_task(&forge_dir, "T-001", "Bug fix", "fix");
        create_completed_task(&forge_dir, "T-002", "Another fix", "fix");
        assert!(matches!(
            suggest_version_bump(&forge_dir),
            VersionBump::Patch
        ));
    }

    #[test]
    fn test_suggest_version_bump_empty() {
        let (_tmp, forge_dir) = setup_forge_dir();
        assert!(matches!(
            suggest_version_bump(&forge_dir),
            VersionBump::None
        ));
    }

    #[test]
    fn test_clean_state() {
        let (_tmp, forge_dir) = setup_forge_dir();
        std::fs::write(forge_dir.join("tasks/T-001.json"), "{}").unwrap();

        // Create a minimal state.json
        let state_mgr = crate::core::state::StateManager::new(&forge_dir);
        let state = crate::core::state::ForgeState {
            project_name: "test".into(),
            current_phase: Some("complete".into()),
            ..Default::default()
        };
        state_mgr.save(&state).unwrap();

        clean_state(&forge_dir).unwrap();

        let cleaned = state_mgr.load().unwrap();
        assert!(cleaned.current_phase.is_none());
        assert!(!forge_dir.join("tasks/T-001.json").exists());
    }
}
