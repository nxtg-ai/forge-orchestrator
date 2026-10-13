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

fn tool_call(name: &str, arguments: serde_json::Value) -> String {
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });
    format!("{init}\n{call}\n")
}

fn tool_text(output: &str) -> String {
    let resp: serde_json::Value = serde_json::from_str(&last_response(output)).unwrap();
    assert_ne!(resp["result"]["isError"], true, "tool error: {resp}");
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

/// DIRECTIVE-NXTG-20261013-07 (alpha run 3 F-L): the outcome of a completed task is readable
/// through `forge_get_tasks`, byte-equal, with the agent that completed it.
#[test]
fn test_mcp_get_tasks_returns_outcome_and_completing_agent() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);
    let summary = "Added four doctest examples to naturalsize — 10/10 pass; \"quoted\", back\\slash, ünïcode ✓";

    let claim = mcp_call(
        &dir,
        &tool_call(
            "forge_claim_task",
            serde_json::json!({"task_id": "T-001", "agent": "claude"}),
        ),
    );
    assert!(tool_text(&claim).contains("claimed by claude"));

    let complete = mcp_call(
        &dir,
        &tool_call(
            "forge_complete_task",
            serde_json::json!({"task_id": "T-001", "result_summary": summary}),
        ),
    );
    assert!(tool_text(&complete).contains("T-001 completed"));

    let list = mcp_call(&dir, &tool_call("forge_get_tasks", serde_json::json!({})));
    let tasks: serde_json::Value = serde_json::from_str(&tool_text(&list)).unwrap();
    let t1 = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "T-001")
        .unwrap();
    assert_eq!(t1["status"], "completed");
    assert_eq!(
        t1["result_summary"].as_str().unwrap().as_bytes(),
        summary.as_bytes()
    );
    assert_eq!(t1["completed_by"], "claude");
    assert!(t1["completed_at"].is_string());

    // Not yet completed: outcome fields present and null.
    let t2 = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "T-002")
        .unwrap();
    assert!(t2["result_summary"].is_null() && t2["completed_by"].is_null());

    // The task's markdown view shows the outcome too.
    let md = std::fs::read_to_string(dir.path().join(".forge/tasks/T-001.md")).unwrap();
    assert!(
        md.contains("## Outcome")
            && md.contains("**Completed by:** claude")
            && md.contains(summary)
    );

    // The completion event now names the agent.
    let events = std::fs::read_to_string(dir.path().join(".forge/events.jsonl")).unwrap();
    let ev: serde_json::Value = events
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|e| e["event_type"] == "task_completed" && e["task_id"] == "T-001")
        .expect("task_completed event");
    assert_eq!(ev["agent"], "claude");
}

/// An explicit `agent` on completion overrides the claimer.
#[test]
fn test_mcp_complete_task_explicit_agent() {
    let dir = TempDir::new().unwrap();
    setup_project_with_tasks(&dir);
    mcp_call(
        &dir,
        &tool_call(
            "forge_claim_task",
            serde_json::json!({"task_id": "T-001", "agent": "claude"}),
        ),
    );
    mcp_call(
        &dir,
        &tool_call(
            "forge_complete_task",
            serde_json::json!({"task_id": "T-001", "agent": "codex", "result_summary": "done by codex"}),
        ),
    );
    let list = mcp_call(
        &dir,
        &tool_call(
            "forge_get_tasks",
            serde_json::json!({"status": "completed"}),
        ),
    );
    let tasks: serde_json::Value = serde_json::from_str(&tool_text(&list)).unwrap();
    assert_eq!(tasks["count"], 1);
    assert_eq!(tasks["tasks"][0]["completed_by"], "codex");
    assert_eq!(tasks["tasks"][0]["result_summary"], "done by codex");

    // An invalid agent is refused, not silently ignored.
    let bad = mcp_call(
        &dir,
        &tool_call(
            "forge_complete_task",
            serde_json::json!({"task_id": "T-002", "agent": "nobody"}),
        ),
    );
    assert!(last_response(&bad).contains("Invalid agent"));
}
