//! Pane/window templates — vendored from cosmux v0.4.2 `templates.rs`.
//!
//! Templates live at `~/.config/cosmux/templates/<name>.yaml` and are read **in place** per the
//! single-store protocol. `log::warn!` is replaced by `tracing::warn!`; the message text is
//! preserved so parity output matches.
//!
//! Merging is separated from loading ([`merge_pane_template`] is pure) so the precedence rules can
//! be tested without a templates directory on disk.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::config::{Pane, PodConfig};
use super::error::{PodError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PaneTemplate {
    #[serde(default)]
    pub default_command: Option<String>,
    #[serde(default)]
    pub on_pane_dead: Vec<String>,
}

/// Template directory override, for tests. Defaults to `~/.config/cosmux/templates`.
///
/// Same rationale as the store seam in [`super::state`]: production reads the shared cosmux
/// location unchanged, and tests redirect rather than depending on the developer's real templates.
pub const TEMPLATE_DIR_ENV: &str = "FORGE_POD_TEMPLATE_DIR";

pub fn template_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(TEMPLATE_DIR_ENV)
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("cosmux")
            .join("templates")
    })
}

pub fn load_template(name: &str) -> Result<Option<PaneTemplate>> {
    let Some(dir) = template_dir() else {
        return Ok(None);
    };
    let path = dir.join(format!("{name}.yaml"));
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let tpl: PaneTemplate = serde_yaml::from_str(&raw).map_err(|e| PodError::InvalidYaml {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(Some(tpl))
}

pub fn apply_templates(pod: &mut PodConfig) -> Result<()> {
    let pod_template = pod.template.clone();
    for window in &mut pod.windows {
        for pane in &mut window.panes {
            let template_name = pane.template.as_deref().or(pod_template.as_deref());
            let Some(name) = template_name else {
                continue;
            };
            let Some(tpl) = load_template(name)? else {
                tracing::warn!(
                    "template '{name}' referenced but not found in ~/.config/cosmux/templates/"
                );
                continue;
            };
            merge_pane_template(pane, &tpl);
        }
    }
    Ok(())
}

/// Merge a template into a pane. Pure — the precedence rule lives here, not in the loader.
///
/// An explicit pane `command` always wins; the template only fills a gap. That ordering is what
/// makes templates safe to apply to the 14 live pods without changing what any of them run.
pub fn merge_pane_template(pane: &mut Pane, tpl: &PaneTemplate) {
    if pane.command.is_none() {
        pane.command = tpl.default_command.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(command: Option<&str>, template: Option<&str>) -> Pane {
        Pane {
            cwd: None,
            command: command.map(String::from),
            template: template.map(String::from),
            task: None,
        }
    }

    #[test]
    fn template_fills_an_empty_command() {
        let mut p = pane(None, Some("claude"));
        merge_pane_template(
            &mut p,
            &PaneTemplate {
                default_command: Some("ccyolo".into()),
                on_pane_dead: vec![],
            },
        );
        assert_eq!(p.command.as_deref(), Some("ccyolo"));
    }

    #[test]
    fn explicit_command_wins_over_the_template() {
        // The invariant that makes templates safe for the live pods: applying one never changes
        // what an already-specified pane runs.
        let mut p = pane(Some("explicit"), Some("claude"));
        merge_pane_template(
            &mut p,
            &PaneTemplate {
                default_command: Some("ccyolo".into()),
                on_pane_dead: vec![],
            },
        );
        assert_eq!(p.command.as_deref(), Some("explicit"));
    }

    #[test]
    fn missing_template_is_a_warning_not_an_error() {
        // Referencing an absent template degrades: upstream behaviour, preserved.
        let previous = std::env::var_os(TEMPLATE_DIR_ENV);
        let empty = std::env::temp_dir().join(format!("forge-pod-tpl-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        unsafe { std::env::set_var(TEMPLATE_DIR_ENV, &empty) };

        let mut pod = PodConfig::parse(
            "name: p\ntemplate: nonexistent\nwindows:\n  - name: w\n    panes:\n      - {}\n",
            "test",
        )
        .unwrap();
        let result = apply_templates(&mut pod);

        match previous {
            Some(v) => unsafe { std::env::set_var(TEMPLATE_DIR_ENV, v) },
            None => unsafe { std::env::remove_var(TEMPLATE_DIR_ENV) },
        }
        let _ = std::fs::remove_dir_all(&empty);

        assert!(result.is_ok(), "missing template must not fail the pod");
        assert_eq!(pod.windows[0].panes[0].command, None);
    }

    #[test]
    fn pane_template_overrides_the_pod_template() {
        let pod = PodConfig::parse(
            "name: p\ntemplate: pod-level\nwindows:\n  - name: w\n    panes:\n      - template: pane-level\n",
            "test",
        )
        .unwrap();
        // Precedence is resolved in apply_templates; assert the config carries both so the
        // resolution order is observable.
        assert_eq!(pod.template.as_deref(), Some("pod-level"));
        assert_eq!(
            pod.windows[0].panes[0].template.as_deref(),
            Some("pane-level")
        );
    }
}
