use crate::mcp::server;
use std::path::Path;

pub fn execute(project_root: &Path) -> anyhow::Result<()> {
    server::run_stdio(project_root)
}
