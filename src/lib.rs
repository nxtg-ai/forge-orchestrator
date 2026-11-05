//! # forge-orchestrator
//!
//! The library behind the `forge` binary. Its documented, supported surface is [`core`]: the types
//! that read and write a project's `.forge/` directory (state and file locks, tasks, the event log,
//! the knowledge base and the plan). Other tools can use it to inspect or drive a Forge project
//! without shelling out to `forge`.
//!
//! The other modules are the binary's internals. They are public only so the binary can use them,
//! are hidden from these docs, and may change in any release.
//!
//! The crate is at the alpha stage, like the binary: the API may change without notice.
//!
//! ```
//! use forge_orchestrator::core::knowledge::{KnowledgeCategory, KnowledgeEntry, KnowledgeManager};
//! use forge_orchestrator::core::task::{Task, TaskManager, TaskStatus};
//!
//! # fn main() -> anyhow::Result<()> {
//! let project = tempfile::tempdir()?;
//! let forge_dir = project.path().join(".forge");
//!
//! // Create a task, then read the board back.
//! let tasks = TaskManager::new(&forge_dir);
//! tasks.create_task(&Task::new("T-001", "Parse the config", "Read forge.toml into Config"))?;
//! let pending: Vec<Task> = tasks
//!     .list_tasks()?
//!     .into_iter()
//!     .filter(|t| t.status == TaskStatus::Pending)
//!     .collect();
//! assert_eq!(pending[0].id, "T-001");
//!
//! // Capture a decision in the knowledge base and list that category.
//! let knowledge = KnowledgeManager::new(&forge_dir);
//! let entry = KnowledgeEntry::new("Use TOML", "Config is TOML, not YAML.", &KnowledgeCategory::Decision)
//!     .with_task("T-001");
//! knowledge.capture(&entry)?;
//! assert_eq!(knowledge.list(Some("decisions"))?.len(), 1);
//! # Ok(())
//! # }
//! ```
#![allow(dead_code)] // Phase 1: many trait methods designed for future phases

pub mod core;

#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod adapters;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod brain;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod cli;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod detect;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod fleet;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod mcp;
// `pod` speaks tmux + flock(2) directly (Unix-only). Windows gets no pod surface — the `forge pod`
// dispatch is stubbed there (see main.rs). Gating the whole module keeps the Windows target building
// (v1.5.2 shipped it); the Unix codepaths are byte-identical, so the §1.5 rollback contract is
// unchanged (DIRECTIVE-NXTG-20260719-20 follow-up: pod is Unix-only by nature).
#[cfg(unix)]
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod pod;
#[doc(hidden)]
#[allow(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
// internals; not rendered
pub mod tui;
