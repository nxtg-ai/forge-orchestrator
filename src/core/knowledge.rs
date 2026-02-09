use crate::brain::KnowledgeCategory;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub category: String,
    pub title: String,
    pub content: String,
    pub source: Option<String>,
    pub task_id: Option<String>,
    pub agent: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl KnowledgeEntry {
    pub fn new(
        title: impl Into<String>,
        content: impl Into<String>,
        category: &KnowledgeCategory,
    ) -> Self {
        let now = Utc::now();
        let id = format!(
            "K-{}-{}",
            now.format("%Y%m%d%H%M%S"),
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        Self {
            id,
            category: category.directory_name().to_string(),
            title: title.into(),
            content: content.into(),
            source: None,
            task_id: None,
            agent: None,
            tags: Vec::new(),
            created_at: now,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Manages the `.forge/knowledge/` directory — captures, classifies, and queries learnings.
pub struct KnowledgeManager {
    forge_dir: PathBuf,
}

impl KnowledgeManager {
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    fn knowledge_dir(&self) -> PathBuf {
        self.forge_dir.join("knowledge")
    }

    /// Store a knowledge entry — writes both .md and .json files
    pub fn capture(&self, entry: &KnowledgeEntry) -> anyhow::Result<()> {
        let cat_dir = self.knowledge_dir().join(&entry.category);
        std::fs::create_dir_all(&cat_dir)?;

        // Write JSON (machine-readable)
        let json_path = cat_dir.join(format!("{}.json", entry.id));
        let json = serde_json::to_string_pretty(entry)?;
        std::fs::write(&json_path, json)?;

        // Write Markdown (human-readable)
        let md_path = cat_dir.join(format!("{}.md", entry.id));
        let md = format_entry_markdown(entry);
        std::fs::write(&md_path, md)?;

        Ok(())
    }

    /// List all knowledge entries, optionally filtered by category
    pub fn list(&self, category: Option<&str>) -> anyhow::Result<Vec<KnowledgeEntry>> {
        let knowledge_dir = self.knowledge_dir();
        if !knowledge_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();

        let dirs_to_scan: Vec<PathBuf> = if let Some(cat) = category {
            vec![knowledge_dir.join(cat)]
        } else {
            std::fs::read_dir(&knowledge_dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        };

        for dir in dirs_to_scan {
            if !dir.exists() {
                continue;
            }
            for file in std::fs::read_dir(&dir)? {
                let file = file?;
                let path = file.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(entry) = serde_json::from_str::<KnowledgeEntry>(&content) {
                            entries.push(entry);
                        }
                    }
                }
            }
        }

        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // newest first
        Ok(entries)
    }

    /// Search knowledge entries by keyword (searches title, content, and tags)
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<KnowledgeEntry>> {
        let all = self.list(None)?;
        let query_lower = query.to_lowercase();

        let results: Vec<KnowledgeEntry> = all
            .into_iter()
            .filter(|e| {
                e.title.to_lowercase().contains(&query_lower)
                    || e.content.to_lowercase().contains(&query_lower)
                    || e.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect();

        Ok(results)
    }

    /// Get a specific knowledge entry by ID
    pub fn get(&self, id: &str) -> anyhow::Result<Option<KnowledgeEntry>> {
        let all = self.list(None)?;
        Ok(all.into_iter().find(|e| e.id == id))
    }

    /// Generate a SKILL.md file from all knowledge in a category
    pub fn generate_skill_md(&self, category: &str) -> anyhow::Result<String> {
        let entries = self.list(Some(category))?;

        let mut content = String::new();
        content.push_str(&format!(
            "# {} — Project Knowledge\n\n",
            capitalize(category)
        ));
        content.push_str(&format!(
            "*Auto-generated by Forge Orchestrator on {}*\n\n",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        ));
        content.push_str("---\n\n");

        if entries.is_empty() {
            content.push_str("No knowledge captured yet in this category.\n");
            return Ok(content);
        }

        for entry in &entries {
            content.push_str(&format!("## {}\n\n", entry.title));
            content.push_str(&format!("{}\n\n", entry.content));

            if let Some(ref source) = entry.source {
                content.push_str(&format!("*Source: {source}*\n\n"));
            }
            if let Some(ref task_id) = entry.task_id {
                content.push_str(&format!("*Related task: {task_id}*\n\n"));
            }
            if !entry.tags.is_empty() {
                content.push_str(&format!("*Tags: {}*\n\n", entry.tags.join(", ")));
            }
            content.push_str("---\n\n");
        }

        Ok(content)
    }

    /// Generate SKILL.md files for all categories and write to .forge/knowledge/
    pub fn generate_all_skills(&self) -> anyhow::Result<Vec<String>> {
        let categories = ["research", "decisions", "learnings", "patterns"];
        let mut generated = Vec::new();

        for cat in &categories {
            let entries = self.list(Some(cat))?;
            if entries.is_empty() {
                continue;
            }

            let skill_content = self.generate_skill_md(cat)?;
            let skill_path = self
                .knowledge_dir()
                .join(format!("{cat}-SKILL.md"));
            std::fs::write(&skill_path, skill_content)?;
            generated.push(format!("{cat}-SKILL.md"));
        }

        Ok(generated)
    }
}

fn format_entry_markdown(entry: &KnowledgeEntry) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {}\n\n", entry.title));
    md.push_str(&format!("**Category:** {}\n", entry.category));
    md.push_str(&format!(
        "**Created:** {}\n",
        entry.created_at.format("%Y-%m-%d %H:%M UTC")
    ));
    if let Some(ref source) = entry.source {
        md.push_str(&format!("**Source:** {source}\n"));
    }
    if let Some(ref task_id) = entry.task_id {
        md.push_str(&format!("**Task:** {task_id}\n"));
    }
    if let Some(ref agent) = entry.agent {
        md.push_str(&format!("**Agent:** {agent}\n"));
    }
    if !entry.tags.is_empty() {
        md.push_str(&format!("**Tags:** {}\n", entry.tags.join(", ")));
    }
    md.push_str(&format!("\n---\n\n{}\n", entry.content));
    md
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_capture_and_list() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let mgr = KnowledgeManager::new(&forge_dir);
        let entry = KnowledgeEntry::new(
            "Rust is fast",
            "We discovered that Rust compiles to fast binaries.",
            &KnowledgeCategory::Research,
        );
        mgr.capture(&entry).unwrap();

        let entries = mgr.list(None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Rust is fast");
        assert_eq!(entries[0].category, "research");
    }

    #[test]
    fn test_capture_with_metadata() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let mgr = KnowledgeManager::new(&forge_dir);
        let entry = KnowledgeEntry::new(
            "Use JWT for auth",
            "We decided to use JWT tokens for authentication.",
            &KnowledgeCategory::Decision,
        )
        .with_source("team discussion")
        .with_task("T-001")
        .with_agent("claude")
        .with_tags(vec!["auth".into(), "jwt".into()]);

        mgr.capture(&entry).unwrap();

        let entries = mgr.list(Some("decisions")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source.as_deref(), Some("team discussion"));
        assert_eq!(entries[0].task_id.as_deref(), Some("T-001"));
        assert_eq!(entries[0].tags, vec!["auth", "jwt"]);
    }

    #[test]
    fn test_search() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let mgr = KnowledgeManager::new(&forge_dir);

        mgr.capture(&KnowledgeEntry::new(
            "JWT tokens expire",
            "JWT tokens should have a 15-minute expiry with refresh tokens.",
            &KnowledgeCategory::Learning,
        ))
        .unwrap();

        mgr.capture(&KnowledgeEntry::new(
            "PostgreSQL indexes",
            "Add indexes on foreign keys for join performance.",
            &KnowledgeCategory::Pattern,
        ))
        .unwrap();

        let results = mgr.search("jwt").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "JWT tokens expire");

        let results = mgr.search("index").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "PostgreSQL indexes");
    }

    #[test]
    fn test_generate_skill_md() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let mgr = KnowledgeManager::new(&forge_dir);

        mgr.capture(&KnowledgeEntry::new(
            "Error handling pattern",
            "Use Result types instead of exceptions.",
            &KnowledgeCategory::Pattern,
        ))
        .unwrap();

        mgr.capture(&KnowledgeEntry::new(
            "Naming convention",
            "Use snake_case for functions, PascalCase for types.",
            &KnowledgeCategory::Pattern,
        ))
        .unwrap();

        let skill = mgr.generate_skill_md("patterns").unwrap();
        assert!(skill.contains("Patterns — Project Knowledge"));
        assert!(skill.contains("Error handling pattern"));
        assert!(skill.contains("Naming convention"));
    }

    #[test]
    fn test_filter_by_category() {
        let dir = TempDir::new().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let mgr = KnowledgeManager::new(&forge_dir);

        mgr.capture(&KnowledgeEntry::new(
            "Research entry",
            "Some research.",
            &KnowledgeCategory::Research,
        ))
        .unwrap();

        mgr.capture(&KnowledgeEntry::new(
            "Decision entry",
            "Some decision.",
            &KnowledgeCategory::Decision,
        ))
        .unwrap();

        assert_eq!(mgr.list(Some("research")).unwrap().len(), 1);
        assert_eq!(mgr.list(Some("decisions")).unwrap().len(), 1);
        assert_eq!(mgr.list(None).unwrap().len(), 2);
    }
}
