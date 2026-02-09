use crate::core::state::DetectedTool;
use crate::core::task::AgentType;
use std::process::Command;

/// Detect which AI CLI tools are installed on this machine
pub fn detect_tools() -> Vec<DetectedTool> {
    let mut tools = Vec::new();

    // Detect Claude Code
    if let Some(tool) = detect_command("claude", &["--version"], AgentType::Claude) {
        tools.push(tool);
    }

    // Detect Codex CLI
    if let Some(tool) = detect_command("codex", &["--version"], AgentType::Codex) {
        tools.push(tool);
    }

    // Detect Gemini CLI
    if let Some(tool) = detect_command("gemini", &["--version"], AgentType::Gemini) {
        tools.push(tool);
    }

    tools
}

fn detect_command(command: &str, args: &[&str], agent_type: AgentType) -> Option<DetectedTool> {
    // First check if the command exists via `which`
    let which_result = Command::new("which").arg(command).output().ok()?;

    if !which_result.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&which_result.stdout)
        .trim()
        .to_string();

    // Try to get version
    let version = Command::new(command)
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(
                    String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .to_string(),
                )
            } else {
                None
            }
        });

    Some(DetectedTool {
        name: command.to_string(),
        agent_type,
        version,
        path,
        available: true,
    })
}

/// Pretty-print detected tools for the init flow
pub fn display_detected_tools(tools: &[DetectedTool]) {
    use colored::Colorize;

    println!("{}", "Detected AI Tools:".bold());
    if tools.is_empty() {
        println!("  {} No AI CLI tools found.", "!".yellow());
        println!("  Install one of: claude, codex, gemini");
        return;
    }

    for tool in tools {
        let status = if tool.available {
            "✓".green().to_string()
        } else {
            "✗".red().to_string()
        };
        let version_str = tool
            .version
            .as_deref()
            .unwrap_or("unknown version");
        println!("  {status} {} ({version_str})", tool.name.cyan());
    }
}
