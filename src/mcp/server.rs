use serde_json::{json, Value};
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
pub fn run_stdio(project_root: &Path) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    let mut line = String::new();

    // Log to stderr so it doesn't interfere with the JSON-RPC protocol on stdout
    eprintln!("[forge-mcp] Server starting for project: {}", project_root.display());

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

        // Handle the request
        let response = handle_request(&request, project_root);

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

fn handle_request(request: &JsonRpcRequest, project_root: &Path) -> Option<JsonRpcResponse> {
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

        let response = handle_request(&request, Path::new("/tmp")).unwrap();
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

        let response = handle_request(&request, Path::new("/tmp")).unwrap();
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 9);
    }

    #[test]
    fn test_handle_ping() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(Value::Number(3.into())),
            method: "ping".into(),
            params: None,
        };

        let response = handle_request(&request, Path::new("/tmp")).unwrap();
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

        let response = handle_request(&request, Path::new("/tmp")).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_notification_returns_none() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None, // Notification — no id
            method: "notifications/initialized".into(),
            params: None,
        };

        let response = handle_request(&request, Path::new("/tmp"));
        assert!(response.is_none());
    }
}
