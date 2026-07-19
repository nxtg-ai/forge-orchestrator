//! Pod YAML config — vendored from cosmux v0.4.2 `config.rs`.
//!
//! Pure parsing and validation: no tmux, no store, no process spawning. The search order and
//! validation messages are preserved verbatim so a vendored `forge pod validate` and a `cosmux
//! validate` agree on the same YAML.
//!
//! Two deviations from upstream, both deliberate:
//!
//! * `shellexpand` is **vendored** rather than added as a dependency — upstream used it at exactly
//!   one call site (`config.rs:110`, `shellexpand::tilde`), which is a few lines of `HOME` lookup.
//! * `Pane` gains an optional `task:` field, the `.forge/` synergy binding. It is
//!   `skip_serializing_if = "Option::is_none"` so forge never writes the field into configs or
//!   store entries that cosmux also reads.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::error::{PodError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodConfig {
    pub name: String,

    #[serde(default)]
    pub root: Option<String>,

    #[serde(default)]
    pub template: Option<String>,

    #[serde(default)]
    pub before_start: Vec<String>,

    #[serde(default)]
    pub after_start: Vec<String>,

    #[serde(default)]
    pub before_attach: Vec<String>,

    #[serde(default)]
    pub after_detach: Vec<String>,

    #[serde(default)]
    pub on_pane_dead: Vec<String>,

    #[serde(default)]
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Window {
    pub name: String,

    #[serde(default = "default_layout")]
    pub layout: String,

    #[serde(default)]
    pub panes: Vec<Pane>,
}

fn default_layout() -> String {
    "tiled".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pane {
    #[serde(default)]
    pub cwd: Option<String>,

    #[serde(default)]
    pub command: Option<String>,

    #[serde(default)]
    pub template: Option<String>,

    /// `.forge/` task binding. When set, dead-pane recovery re-claims this task and appends a
    /// `PaneRecovered` event. Absent in every cosmux-authored pod, and never serialized when
    /// absent, so the shared config/store format is unchanged for cosmux readers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

impl PodConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let path_str = path_ref.display().to_string();
        if !path_ref.exists() {
            return Err(PodError::ConfigNotFound(path_str));
        }
        let raw = std::fs::read_to_string(path_ref)?;
        Self::parse(&raw, &path_str)
    }

    /// Parse and validate YAML without touching the filesystem — the seam the parity fixtures use.
    pub fn parse(raw: &str, path_label: &str) -> Result<Self> {
        let cfg: PodConfig = serde_yaml::from_str(raw).map_err(|e| PodError::InvalidYaml {
            path: path_label.to_string(),
            source: e,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(PodError::InvalidConfig("pod name is required".into()));
        }
        if self.windows.is_empty() {
            return Err(PodError::InvalidConfig(format!(
                "pod '{}' must declare at least one window",
                self.name
            )));
        }
        for w in &self.windows {
            if w.name.is_empty() {
                return Err(PodError::InvalidConfig(format!(
                    "pod '{}' has a window with no name",
                    self.name
                )));
            }
            if w.panes.is_empty() {
                return Err(PodError::InvalidConfig(format!(
                    "window '{}/{}' must declare at least one pane",
                    self.name, w.name
                )));
            }
        }
        Ok(())
    }

    pub fn expanded_root(&self) -> Option<PathBuf> {
        self.root.as_deref().map(expand_path)
    }
}

/// Expand a leading `~` against `$HOME`.
///
/// Vendored replacement for `shellexpand::tilde`, which upstream used at a single call site.
/// Deliberately narrow: only a leading `~` or `~/` is expanded — `~user` is left untouched, as
/// upstream's behaviour for that form was never exercised by any pod config.
pub fn expand_path<S: AsRef<str>>(s: S) -> PathBuf {
    let raw = s.as_ref();
    let Some(home) = home_dir() else {
        return PathBuf::from(raw);
    };
    if raw == "~" {
        return home;
    }
    match raw.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(raw),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Legacy pod search order, preserved verbatim from cosmux `config.rs:114`.
///
/// `~/ASIF/infra/tmux/` comes first: that is where the 14 live fleet pods are, and the
/// single-store protocol requires forge to find them exactly where cosmux does.
pub fn config_search_paths(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        paths.push(
            home.join("ASIF")
                .join("infra")
                .join("tmux")
                .join(format!("{name}.yaml")),
        );
        paths.push(
            home.join(".config")
                .join("cosmux")
                .join("pods")
                .join(format!("{name}.yaml")),
        );
    }
    paths.push(PathBuf::from(format!("./{name}.yaml")));
    paths
}

pub fn resolve_pod_path(name_or_path: &str) -> Result<PathBuf> {
    let direct = PathBuf::from(name_or_path);
    if direct.exists() {
        return Ok(direct);
    }
    for candidate in config_search_paths(name_or_path) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(PodError::ConfigNotFound(format!(
        "no pod config found for '{name_or_path}' (searched ~/ASIF/infra/tmux/, ~/.config/cosmux/pods/, ./)"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
name: test-pod
windows:
  - name: main
    panes:
      - cwd: ~/projects/x
        command: ccyolo
"#;

    #[test]
    fn parses_a_minimal_pod() {
        let cfg = PodConfig::parse(MINIMAL, "test").expect("valid pod");
        assert_eq!(cfg.name, "test-pod");
        assert_eq!(cfg.windows[0].layout, "tiled", "layout defaults to tiled");
        assert_eq!(cfg.windows[0].panes[0].command.as_deref(), Some("ccyolo"));
        assert_eq!(cfg.windows[0].panes[0].task, None);
    }

    #[test]
    fn task_binding_parses_and_is_omitted_when_absent() {
        let with_task = PodConfig::parse(
            r#"
name: t
windows:
  - name: w
    panes:
      - command: x
        task: T-042
"#,
            "test",
        )
        .unwrap();
        assert_eq!(with_task.windows[0].panes[0].task.as_deref(), Some("T-042"));

        // A pod without `task:` must serialize without the key — cosmux reads these files too.
        let plain = PodConfig::parse(MINIMAL, "test").unwrap();
        let out = serde_yaml::to_string(&plain).unwrap();
        assert!(!out.contains("task"), "{out}");
    }

    #[test]
    fn validation_messages_match_upstream() {
        let no_windows = PodConfig::parse("name: p\nwindows: []\n", "test").unwrap_err();
        assert!(
            no_windows
                .to_string()
                .contains("must declare at least one window"),
            "{no_windows}"
        );

        let no_panes = PodConfig::parse("name: p\nwindows:\n  - name: w\n    panes: []\n", "test")
            .unwrap_err();
        assert!(
            no_panes
                .to_string()
                .contains("must declare at least one pane"),
            "{no_panes}"
        );

        let no_name = PodConfig::parse(
            "name: ''\nwindows:\n  - name: w\n    panes:\n      - command: x\n",
            "test",
        )
        .unwrap_err();
        assert!(
            no_name.to_string().contains("pod name is required"),
            "{no_name}"
        );
    }

    #[test]
    fn invalid_yaml_names_the_path() {
        let err = PodConfig::parse("name: [unclosed", "/tmp/broken.yaml").unwrap_err();
        assert!(err.to_string().contains("/tmp/broken.yaml"), "{err}");
    }

    #[test]
    fn expands_tilde_like_shellexpand() {
        let home = std::env::var("HOME").expect("HOME set");
        assert_eq!(expand_path("~"), PathBuf::from(&home));
        assert_eq!(
            expand_path("~/projects/x"),
            PathBuf::from(&home).join("projects/x")
        );
        // Absolute and relative paths pass through untouched.
        assert_eq!(expand_path("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_path("rel/path"), PathBuf::from("rel/path"));
        // `~user` is NOT expanded — documented deviation, unused by any pod config.
        assert_eq!(expand_path("~other/x"), PathBuf::from("~other/x"));
    }

    #[test]
    fn search_order_puts_the_fleet_directory_first() {
        let paths = config_search_paths("mypod");
        assert!(
            paths[0].ends_with("ASIF/infra/tmux/mypod.yaml"),
            "{:?}",
            paths[0]
        );
        assert!(
            paths[1].ends_with(".config/cosmux/pods/mypod.yaml"),
            "{:?}",
            paths[1]
        );
        assert_eq!(paths[2], PathBuf::from("./mypod.yaml"));
    }
}
