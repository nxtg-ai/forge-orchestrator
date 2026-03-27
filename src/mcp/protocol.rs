use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 Request received from an MCP client.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, always `"2.0"`.
    pub jsonrpc: String,
    /// Request identifier. `None` for notifications.
    pub id: Option<Value>,
    /// The RPC method name (e.g., `"tools/call"`, `"initialize"`).
    pub method: String,
    /// Optional parameters for the method.
    #[serde(default)]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 Response sent back to an MCP client.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    /// Protocol version, always `"2.0"`.
    pub jsonrpc: String,
    /// Echoed request identifier. `None` for notification responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// Successful result payload, mutually exclusive with `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error payload, mutually exclusive with `result`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Error object included in error responses.
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    /// Numeric error code (e.g., `-32601` for method not found).
    pub code: i64,
    /// Human-readable error description.
    pub message: String,
    /// Optional structured error data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// Create a successful JSON-RPC response with the given result payload.
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error JSON-RPC response with the given code and message.
    pub fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    /// Create a standard "method not found" (-32601) error response.
    pub fn method_not_found(id: Option<Value>, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }
}

/// MCP Protocol: ServerInfo returned by initialize
#[derive(Debug, Serialize)]
pub struct ServerInfo {
    /// Server name (e.g., `"forge-orchestrator"`).
    pub name: String,
    /// Server version (e.g., `"1.4.1"`).
    pub version: String,
}

/// MCP Protocol: ServerCapabilities advertised during initialization.
#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    /// Tool-related capabilities.
    pub tools: ToolsCapability,
}

/// Capability flags for the tools subsystem.
#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    /// Whether the server supports dynamic tool list change notifications.
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

/// MCP Protocol: InitializeResult returned to the client after handshake.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// MCP protocol version (e.g., `"2024-11-05"`).
    pub protocol_version: String,
    /// Capabilities this server supports.
    pub capabilities: ServerCapabilities,
    /// Identifying information about this server.
    pub server_info: ServerInfo,
}

/// MCP Protocol: Tool definition advertised via `tools/list`.
#[derive(Debug, Serialize, Clone)]
pub struct ToolDefinition {
    /// Unique tool name (e.g., `"forge_get_tasks"`).
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP Protocol: ToolsListResult returned by `tools/list`.
#[derive(Debug, Serialize)]
pub struct ToolsListResult {
    /// All tools exposed by this server.
    pub tools: Vec<ToolDefinition>,
}

/// MCP Protocol: Content block in tool result (currently text-only).
#[derive(Debug, Serialize)]
pub struct ContentBlock {
    /// Content type, always `"text"` for now.
    #[serde(rename = "type")]
    pub content_type: String,
    /// The text content of this block.
    pub text: String,
}

/// MCP Protocol: CallToolResult returned by `tools/call`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    /// One or more content blocks containing the tool's output.
    pub content: Vec<ContentBlock>,
    /// Set to `true` when the tool invocation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    /// Create a successful tool result containing a single text block.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock {
                content_type: "text".into(),
                text: text.into(),
            }],
            is_error: None,
        }
    }

    /// Create an error tool result containing a single text block with `is_error: true`.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock {
                content_type: "text".into(),
                text: text.into(),
            }],
            is_error: Some(true),
        }
    }
}
