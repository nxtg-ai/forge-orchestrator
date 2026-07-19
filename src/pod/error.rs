//! Pod error type — vendored from cosmux v0.4.2 `error.rs`, adapted to `thiserror` 2.
//!
//! Renamed `CosmuxError` → `PodError`; variants and messages are otherwise preserved so parity
//! fixtures can compare user-facing text against cosmux byte-for-byte.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PodError {
    #[error("config file not found: {0}")]
    ConfigNotFound(String),

    #[error("invalid YAML in {path}: {source}")]
    InvalidYaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("invalid pod config: {0}")]
    InvalidConfig(String),

    #[error("tmux command failed: {cmd} (exit {code})\nstderr: {stderr}")]
    TmuxFailed {
        cmd: String,
        code: i32,
        stderr: String,
    },

    #[error("tmux not found on PATH — install tmux first")]
    TmuxNotFound,

    #[error("hook failed: {hook} — {reason}")]
    HookFailed { hook: String, reason: String },

    #[error("template not found: {0}")]
    TemplateNotFound(String),

    #[error("session already exists: {0} (use --force to replace)")]
    SessionExists(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, PodError>;
