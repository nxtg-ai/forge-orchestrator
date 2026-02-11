#![allow(deprecated)] // cargo_bin deprecation — tracked, not blocking

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn forge_cmd() -> Command {
    Command::cargo_bin("forge").unwrap()
}

#[test]
fn test_init_creates_forge_directory() {
    let dir = TempDir::new().unwrap();

    // Initialize git (required for context discovery)
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "Test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Forge initialized for Test"));

    assert!(dir.path().join(".forge").exists());
    assert!(dir.path().join(".forge/state.json").exists());
    assert!(dir.path().join(".forge/events.jsonl").exists());
    assert!(dir.path().join(".forge/governance.json").exists());
    assert!(dir.path().join(".forge/tasks").is_dir());
    assert!(dir.path().join(".forge/knowledge").is_dir());
}

#[test]
fn test_status_without_init_shows_warning() {
    let dir = TempDir::new().unwrap();

    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Forge is not initialized"));
}

#[test]
fn test_status_after_init_shows_dashboard() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Init first
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "StatusTest",
        ])
        .assert()
        .success();

    // Status should show dashboard
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("FORGE ORCHESTRATOR STATUS"))
        .stdout(predicate::str::contains("StatusTest"))
        .stdout(predicate::str::contains("Total: 0"));
}

#[test]
fn test_plan_generate_creates_tasks() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Init
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "PlanTest",
        ])
        .assert()
        .success();

    // Create SPEC.md
    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Spec\n\n## Auth\nBuild auth system\n\n## API\nBuild API layer\n",
    )
    .unwrap();

    // Generate plan
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "plan",
            "--generate",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("CEO Mode"))
        .stdout(predicate::str::contains("Generated 2 tasks"))
        .stdout(predicate::str::contains("T-001"))
        .stdout(predicate::str::contains("T-002"));

    // Verify task files exist
    assert!(dir.path().join(".forge/tasks/T-001.json").exists());
    assert!(dir.path().join(".forge/tasks/T-002.json").exists());
    assert!(dir.path().join(".forge/plan.md").exists());
}

#[test]
fn test_plan_show_template_after_init() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "PlanShow",
        ])
        .assert()
        .success();

    // Init creates a template plan.md, so `forge plan` should show it
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "plan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Master Plan"));
}

#[test]
fn test_sync_reconciles_state() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Init + generate plan
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "SyncTest",
        ])
        .assert()
        .success();

    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Spec\n\n## Feature A\nDo stuff\n\n## Feature B\nDo more\n\n## Feature C\nEven more\n",
    )
    .unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "plan",
            "--generate",
        ])
        .assert()
        .success();

    // Sync
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sync complete"))
        .stdout(predicate::str::contains("3 total, 3 pending"));

    // Verify CLAUDE.md was generated (only when claude CLI is installed)
    let claude_available =
        std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
            .arg("claude")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    if claude_available {
        assert!(dir.path().join("CLAUDE.md").exists());
    }
}

#[test]
fn test_full_loop_init_plan_sync_status() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Step 1: Init
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "LoopTest",
        ])
        .assert()
        .success();

    // Step 2: Create spec
    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Full Loop Spec\n\n## Backend\nNode.js API\n\n## Frontend\nReact SPA\n",
    )
    .unwrap();

    // Step 3: Generate plan
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "plan",
            "--generate",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated 2 tasks"));

    // Step 4: Sync (renders adapter configs)
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sync complete"));

    // Step 5: Status shows tasks
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total: 2"))
        .stdout(predicate::str::contains("Pending: 2"));

    // Step 6: Verify plan is readable
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "plan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("LoopTest — Master Plan"))
        .stdout(predicate::str::contains("Backend"))
        .stdout(predicate::str::contains("Frontend"));

    // Step 7: Verify events were logged
    let events = std::fs::read_to_string(dir.path().join(".forge/events.jsonl")).unwrap();
    let event_count = events.lines().count();
    assert!(
        event_count >= 3,
        "Expected at least 3 events (init + plan + sync), got {event_count}"
    );
}

#[test]
fn test_plan_generate_without_any_context_shows_error() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "NoSpec",
        ])
        .assert()
        .success();

    // No SPEC.md and no README.md → error with guidance
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "plan",
            "--generate",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No project context found"));
}

#[test]
fn test_plan_generate_from_readme_context() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "ContextPlan",
        ])
        .assert()
        .success();

    // Create a README with sections — no SPEC.md
    std::fs::write(
        dir.path().join("README.md"),
        "# ContextPlan\n\n## Authentication\nOAuth2 login flow\n\n## Dashboard\nReal-time metrics\n",
    )
    .unwrap();

    // Should gather README as context and generate tasks from it
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "plan",
            "--generate",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("project context"))
        .stdout(predicate::str::contains("T-001"));

    // Verify task files were created
    assert!(dir.path().join(".forge/tasks/T-001.json").exists());
}

#[test]
fn test_init_detects_context_files() {
    let dir = TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Create various context files
    std::fs::write(dir.path().join("SPEC.md"), "# Spec").unwrap();
    std::fs::write(dir.path().join("package.json"), "{}").unwrap();

    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "ContextTest",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("SPEC.md").and(predicate::str::contains("package.json")));
}
