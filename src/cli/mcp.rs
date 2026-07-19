use crate::mcp::server;
use std::path::Path;

/// Start the stdio MCP server.
///
/// `explicit_binding` is true when the project came from an explicit `--project` or from
/// `FORGE_PROJECT_ROOT` (DIRECTIVE-16). When set, the binding is authoritative for this connection
/// and `forge_set_project` is refused, so no other consumer's global switch can override it.
pub fn execute(project_root: &Path, explicit_binding: bool) -> anyhow::Result<()> {
    server::run_stdio(project_root, explicit_binding)
}
