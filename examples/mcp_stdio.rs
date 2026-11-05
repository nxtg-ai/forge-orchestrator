//! Talk to `forge mcp` over stdio the way an MCP client does: initialize, list the tools, and
//! create a task with `forge_create_task`. The project only needs a `.forge/` directory, which this
//! example creates through the library if it is missing. It starts no AI tool.
//!
//! ```sh
//! cargo build && cargo run --example mcp_stdio -- target/debug/forge path/to/project
//! ```
use forge_orchestrator::core::state::{ForgeState, StateManager};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let forge = args.next().unwrap_or_else(|| "forge".into());
    let project = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let mut server = Command::new(&forge)
        .arg("--project")
        .arg(&project)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    // Only once the server has started: a wrong binary path then leaves no .forge/ behind.
    if !project.join(".forge").exists() {
        StateManager::new(project.join(".forge")).save(&ForgeState::default())?;
    }
    let mut stdin = server.stdin.take().expect("piped stdin");
    let mut stdout = BufReader::new(server.stdout.take().expect("piped stdout"));
    // One JSON-RPC message per line, each way.
    let mut call = |msg: Value| -> anyhow::Result<Option<Value>> {
        writeln!(stdin, "{msg}")?;
        if msg.get("id").is_none() {
            return Ok(None); // a notification gets no reply
        }
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        Ok(Some(serde_json::from_str(&line)?))
    };

    let init = call(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05", "capabilities": {},
        "clientInfo": {"name": "mcp_stdio example", "version": "0"}}}),
    )?
    .expect("reply");
    let info = &init["result"]["serverInfo"];
    println!(
        "server: {} {}",
        info["name"].as_str().unwrap_or("?"),
        info["version"].as_str().unwrap_or("?")
    );
    call(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;

    let list = call(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))?.expect("reply");
    let tools = list["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    println!("{} tools:", tools.len());
    for t in &tools {
        println!("  {}", t["name"].as_str().unwrap_or("?"));
    }

    let created = call(
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "forge_create_task",
        "arguments": {"title": "Parse the config", "description": "Read forge.toml into Config"}}}),
    )?
    .expect("reply");
    let text = created["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let task: Value = serde_json::from_str(text)?;
    println!(
        "created {} ({})",
        task["task_id"].as_str().unwrap_or("?"),
        task["status"].as_str().unwrap_or("?")
    );

    drop(stdin); // EOF ends the server
    server.wait()?;
    Ok(())
}
