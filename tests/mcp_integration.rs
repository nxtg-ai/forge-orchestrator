#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn forge_cmd() -> Command {
    Command::cargo_bin("forge").unwrap()
}

fn setup_project_with_tasks(dir: &TempDir) {
    // Git init
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Forge init
    forge_cmd()
        .args([
            "--project",
            dir.path().to_str().unwrap(),
            "init",
            "--name",
            "MCPTest",
        ])
        .assert()
        .success();

    // Create SPEC and generate plan
    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Spec\n\n## Auth\nBuild auth\n\n## API\nBuild API\n",
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
}

fn mcp_call(dir: &TempDir, input: &str) -> String {
    let output = forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .output()
        .expect("failed to run forge mcp");

    String::from_utf8(output.stdout).unwrap_or_default()
}

fn last_response(output: &str) -> String {
    output.lines().last().unwrap_or("").to_string()
}

#[test]
fn test_mcp_initialize() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("forge-mcp"), "Should contain server name");
    assert!(
        resp.contains("2024-11-05"),
        "Should contain protocol version"
    );
}

#[test]
fn test_mcp_tools_list() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("forge_get_tasks"));
    assert!(resp.contains("forge_claim_task"));
    assert!(resp.contains("forge_complete_task"));
    assert!(resp.contains("forge_get_state"));
    assert!(resp.contains("forge_get_plan"));
    assert!(resp.contains("forge_capture_knowledge"));
    assert!(resp.contains("forge_get_knowledge"));
    assert!(resp.contains("forge_check_drift"));
    assert!(resp.contains("forge_get_health"));
}

#[test]
fn test_mcp_get_tasks() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_tasks","arguments":{}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("T-001"));
    assert!(resp.contains("T-002"));
    assert!(resp.contains("Auth"));
    assert!(resp.contains("API"));
}

#[test]
fn test_mcp_get_state() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_state","arguments":{}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("MCPTest"));
    assert!(resp.contains("task_summary"));
}

#[test]
fn test_mcp_get_plan() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_plan","arguments":{}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("Master Plan"));
    assert!(resp.contains("Auth"));
}

#[test]
fn test_mcp_claim_and_complete_flow() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    // Claim T-001
    let claim_input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_claim_task","arguments":{"task_id":"T-001","agent":"claude"}}}"#,
        "\n",
    );
    let claim_output = mcp_call(&dir, claim_input);
    let claim_resp = last_response(&claim_output);
    assert!(claim_resp.contains("claimed by claude"));

    // Complete T-001
    let complete_input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_complete_task","arguments":{"task_id":"T-001","result_summary":"Auth done"}}}"#,
        "\n",
    );
    let complete_output = mcp_call(&dir, complete_input);
    let complete_resp = last_response(&complete_output);
    assert!(complete_resp.contains("completed"));
    assert!(complete_resp.contains("T-002")); // Newly available

    // Verify status reflects completion
    forge_cmd()
        .args(["--project", dir.path().to_str().unwrap(), "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 done"));
}

#[test]
fn test_mcp_claim_already_claimed_task() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    // Claim T-001 first
    let input1 = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_claim_task","arguments":{"task_id":"T-001","agent":"claude"}}}"#,
        "\n",
    );
    mcp_call(&dir, input1);

    // Try to claim again — should error
    let input2 = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_claim_task","arguments":{"task_id":"T-001","agent":"codex"}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input2);
    let resp = last_response(&output);
    assert!(resp.contains("not pending") || resp.contains("isError"));
}

#[test]
fn test_mcp_ping() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);
    assert!(resp.contains("\"result\":{}") || resp.contains("\"result\": {}"));
}

#[test]
fn test_mcp_capture_and_query_knowledge() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    // Capture a knowledge entry
    let capture_input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_capture_knowledge","arguments":{"title":"JWT tokens expire","content":"Research finding: JWT tokens should have 15-minute expiry with refresh tokens for security.","tags":["auth","jwt","security"]}}}"#,
        "\n",
    );
    let capture_output = mcp_call(&dir, capture_input);
    let capture_resp = last_response(&capture_output);
    assert!(capture_resp.contains("Knowledge captured"));
    assert!(capture_resp.contains("research")); // auto-classified as research

    // Query knowledge
    let query_input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_knowledge","arguments":{"query":"jwt"}}}"#,
        "\n",
    );
    let query_output = mcp_call(&dir, query_input);
    let query_resp = last_response(&query_output);
    assert!(query_resp.contains("JWT tokens expire"));
    assert!(query_resp.contains("count"));
}

#[test]
fn test_mcp_capture_with_explicit_category() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_capture_knowledge","arguments":{"title":"Use Rust for orchestrator","content":"We decided to use Rust for the orchestrator binary.","category":"decisions","source":"team discussion","task_id":"T-001"}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);
    assert!(resp.contains("decisions"));

    // List by category
    let list_input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_knowledge","arguments":{"category":"decisions"}}}"#,
        "\n",
    );
    let list_output = mcp_call(&dir, list_input);
    let list_resp = last_response(&list_output);
    assert!(list_resp.contains("Use Rust for orchestrator"));
}

#[test]
fn test_mcp_get_health() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_get_health","arguments":{}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("health_score"));
    assert!(resp.contains("findings"));
    assert!(resp.contains("summary"));
}

#[test]
fn test_mcp_check_drift() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"forge_check_drift","arguments":{}}}"#,
        "\n",
    );
    let output = mcp_call(&dir, input);
    let resp = last_response(&output);

    assert!(resp.contains("drift"));
    assert!(resp.contains("summary"));
    // SPEC.md exists in test project, so drift should have vision_alignment
    assert!(resp.contains("vision_alignment"));
}
