use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::Path;

use super::protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, ServerCapabilities, ServerInfo,
    ToolsCapability, ToolsListResult,
};
use super::tools;

/// Run the MCP server over stdio (stdin/stdout).
///
/// Protocol: JSON-RPC 2.0, one message per line.
/// Handles: initialize, notifications/initialized, tools/list, tools/call
pub fn run_stdio(project_root: &Path, explicit_binding: bool) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    let mut line = String::new();
    let mut current_project = project_root.to_path_buf();

    // Log to stderr so it doesn't interfere with the JSON-RPC protocol on stdout
    eprintln!(
        "[forge-mcp] Server starting for project: {} (binding: {})",
        project_root.display(),
        if explicit_binding {
            "explicit — set_project disabled"
        } else {
            "default — set_project enabled"
        }
    );

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            // EOF — client disconnected
            eprintln!("[forge-mcp] Client disconnected (EOF)");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse JSON-RPC request
        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(e) => {
                let response = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
                send_response(&mut writer, &response)?;
                continue;
            }
        };

        // Handle the request (may mutate current_project for set_project, unless bound)
        let response = handle_request(&request, &mut current_project, explicit_binding);

        // Notifications (no id) don't get responses
        if request.id.is_none() {
            eprintln!("[forge-mcp] Notification: {}", request.method);
            continue;
        }

        if let Some(response) = response {
            send_response(&mut writer, &response)?;
        }
    }

    Ok(())
}

fn handle_request(
    request: &JsonRpcRequest,
    project_root: &mut std::path::PathBuf,
    explicit_binding: bool,
) -> Option<JsonRpcResponse> {
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => {
            let result = InitializeResult {
                protocol_version: "2024-11-05".into(),
                capabilities: ServerCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
                server_info: ServerInfo {
                    name: "forge-mcp".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            };
            eprintln!("[forge-mcp] Initialized (protocol: 2024-11-05)");
            Some(JsonRpcResponse::success(
                id,
                serde_json::to_value(result).unwrap(),
            ))
        }

        "notifications/initialized" => {
            // Acknowledgement notification — no response needed
            None
        }

        "tools/list" => {
            let result = ToolsListResult {
                tools: tools::list_tools(),
            };
            Some(JsonRpcResponse::success(
                id,
                serde_json::to_value(result).unwrap(),
            ))
        }

        "tools/call" => {
            let params = request.params.as_ref();

            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let arguments = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));

            eprintln!("[forge-mcp] Calling tool: {tool_name}");

            // Special handling for forge_set_project — mutates server state.
            //
            // DIRECTIVE-16: an explicit binding (`--project` / `FORGE_PROJECT_ROOT`) is
            // authoritative for this connection. `set_project` MUST NOT override it, or a second
            // consumer sharing the server could repoint this one's health/state at its own project.
            // A server started unbound (default cwd) still allows switching — global active-project
            // as default-when-unspecified only.
            if tool_name == "forge_set_project" {
                let result = if explicit_binding {
                    eprintln!(
                        "[forge-mcp] Refused set_project: project is explicitly bound to {}",
                        project_root.display()
                    );
                    super::protocol::CallToolResult::error(format!(
                        "Project is explicitly bound to {} via --project/FORGE_PROJECT_ROOT and \
                         cannot be switched at runtime. Start the server without an explicit \
                         binding to enable forge_set_project.",
                        project_root.display()
                    ))
                } else {
                    match tools::handle_set_project(&arguments) {
                        Ok(new_path) => {
                            eprintln!("[forge-mcp] Project switched to: {}", new_path.display());
                            *project_root = new_path.clone();
                            super::protocol::CallToolResult::text(format!(
                                "Project switched to: {}",
                                new_path.display()
                            ))
                        }
                        Err(err_result) => err_result,
                    }
                };
                return Some(JsonRpcResponse::success(
                    id,
                    serde_json::to_value(result).unwrap(),
                ));
            }

            let result = tools::call_tool(tool_name, &arguments, project_root);
            Some(JsonRpcResponse::success(
                id,
                serde_json::to_value(result).unwrap(),
            ))
        }

        "ping" => Some(JsonRpcResponse::success(id, json!({}))),

        _ => {
            eprintln!("[forge-mcp] Unknown method: {}", request.method);
            Some(JsonRpcResponse::method_not_found(id, &request.method))
        }
    }
}

fn send_response(writer: &mut impl Write, response: &JsonRpcResponse) -> io::Result<()> {
    let json = serde_json::to_string(response).unwrap();
    writeln!(writer, "{json}")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_initialize() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(1.into())),
            method: "initialize".into(),
            params: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1.0"}
            })),
        };

        let mut root = std::path::PathBuf::from("/tmp");
        let response = handle_request(&request, &mut root, false).unwrap();
        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "forge-mcp");
    }

    #[test]
    fn test_handle_tools_list() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(2.into())),
            method: "tools/list".into(),
            params: None,
        };

        let mut root = std::path::PathBuf::from("/tmp");
        let response = handle_request(&request, &mut root, false).unwrap();
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 11);
    }

    #[test]
    fn test_handle_ping() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(3.into())),
            method: "ping".into(),
            params: None,
        };

        let mut root = std::path::PathBuf::from("/tmp");
        let response = handle_request(&request, &mut root, false).unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_handle_unknown_method() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(4.into())),
            method: "unknown/method".into(),
            params: None,
        };

        let mut root = std::path::PathBuf::from("/tmp");
        let response = handle_request(&request, &mut root, false).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    fn set_project_request(path: &str) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(9.into())),
            method: "tools/call".into(),
            params: Some(json!({
                "name": "forge_set_project",
                "arguments": {"path": path},
            })),
        }
    }

    #[test]
    fn set_project_is_refused_and_leaves_the_root_unchanged_under_an_explicit_binding() {
        // DIRECTIVE-16: an explicitly bound server must not let set_project repoint it — that is
        // the exact override another consumer used to contaminate this connection.
        let bound = std::path::PathBuf::from("/tmp/projA");
        let mut root = bound.clone();
        let response = handle_request(&set_project_request("/tmp/projB"), &mut root, true).unwrap();

        assert_eq!(
            root, bound,
            "an explicit binding must not be mutated by set_project"
        );
        let text = response.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("explicitly bound"), "{text}");
        assert!(text.contains("cannot be switched"), "{text}");
    }

    #[test]
    fn set_project_still_switches_when_the_server_is_unbound() {
        // The single-project UX default the DoD protects: a server started without an explicit
        // binding may still switch (global active-project as default-when-unspecified only).
        let tmp = std::env::temp_dir().join(format!("forge-mcp-bind-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join(".forge")).unwrap();
        let mut root = std::path::PathBuf::from("/tmp/does-not-matter");
        let response = handle_request(
            &set_project_request(&tmp.display().to_string()),
            &mut root,
            false,
        )
        .unwrap();

        assert_eq!(root, tmp, "an unbound server must honour set_project");
        let text = response.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("Project switched to"), "{text}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_notification_returns_none() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None, // Notification — no id
            method: "notifications/initialized".into(),
            params: None,
        };

        let mut root = std::path::PathBuf::from("/tmp");
        let response = handle_request(&request, &mut root, false);
        assert!(response.is_none());
    }
}
