//! Capture a decision in a project's knowledge base, then list that category.
//!
//! ```sh
//! cargo run --example capture_knowledge -- path/to/project
//! ```
use forge_orchestrator::core::knowledge::{KnowledgeCategory, KnowledgeEntry, KnowledgeManager};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let project = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let knowledge = KnowledgeManager::new(project.join(".forge"));
    let entry = KnowledgeEntry::new(
        "Config format",
        "Project config is TOML (forge.toml), not YAML.",
        &KnowledgeCategory::Decision,
    )
    .with_tags(vec!["config".into()]);
    knowledge.capture(&entry)?;
    println!("captured {} in decisions/", entry.id);

    let decisions = knowledge.list(Some(KnowledgeCategory::Decision.directory_name()))?;
    println!("decisions: {}", decisions.len());
    for d in decisions {
        println!("  {} {}", d.id, d.title);
    }
    Ok(())
}
