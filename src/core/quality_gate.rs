use std::path::Path;
use std::time::Instant;

/// A quality check to run between build phases.
#[derive(Debug, Clone)]
pub struct QualityGate {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// Result of running a quality gate.
#[derive(Debug, Clone)]
pub struct GateResult {
    pub gate_name: String,
    pub passed: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Auto-detect quality gates from project files in the given root directory.
pub fn detect_gates(project_root: &Path) -> Vec<QualityGate> {
    let mut gates = Vec::new();

    // Rust projects
    if project_root.join("Cargo.toml").exists() {
        gates.push(QualityGate {
            name: "Cargo Test".into(),
            command: "cargo".into(),
            args: vec!["test".into()],
        });
        gates.push(QualityGate {
            name: "Cargo Clippy".into(),
            command: "cargo".into(),
            args: vec![
                "clippy".into(),
                "--".into(),
                "-W".into(),
                "clippy::all".into(),
            ],
        });
    }

    // Node.js projects
    if project_root.join("package.json").exists()
        && let Ok(content) = std::fs::read_to_string(project_root.join("package.json"))
        && let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let scripts = pkg.get("scripts").and_then(|s| s.as_object());

        // TypeCheck: prefer scripts.typecheck, fallback to tsconfig.json
        if scripts.is_some_and(|s| s.contains_key("typecheck")) {
            gates.push(QualityGate {
                name: "TypeCheck".into(),
                command: "npm".into(),
                args: vec!["run".into(), "typecheck".into()],
            });
        } else if project_root.join("tsconfig.json").exists() {
            gates.push(QualityGate {
                name: "TypeCheck".into(),
                command: "npx".into(),
                args: vec!["tsc".into(), "--noEmit".into()],
            });
        }

        // Test suite (--run prevents watch mode in Vitest/Jest)
        if scripts.is_some_and(|s| s.contains_key("test")) {
            gates.push(QualityGate {
                name: "Test Suite".into(),
                command: "npm".into(),
                args: vec!["test".into(), "--".into(), "--run".into()],
            });
        }

        // Lint
        if scripts.is_some_and(|s| s.contains_key("lint")) {
            gates.push(QualityGate {
                name: "Lint".into(),
                command: "npm".into(),
                args: vec!["run".into(), "lint".into()],
            });
        }
    }

    // Python projects
    if project_root.join("pyproject.toml").exists() || project_root.join("setup.py").exists() {
        gates.push(QualityGate {
            name: "Pytest".into(),
            command: "python".into(),
            args: vec!["-m".into(), "pytest".into()],
        });
    }

    // Playwright E2E tests
    if project_root.join("playwright.config.ts").exists()
        || project_root.join("playwright.config.js").exists()
    {
        gates.push(QualityGate {
            name: "Playwright E2E".into(),
            command: "npx".into(),
            args: vec!["playwright".into(), "test".into(), "--reporter=list".into()],
        });
    }

    gates
}

/// Keep the last `max` characters of a string, prepending a truncation marker.
fn truncate_tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("...truncated...\n{}", &s[s.len() - max..])
    }
}

/// Run all gates sequentially and return results. This is blocking — call from a
/// background thread to avoid stalling the TUI.
pub fn run_gates(project_root: &Path, gates: &[QualityGate]) -> Vec<GateResult> {
    gates
        .iter()
        .map(|gate| run_single_gate(project_root, gate))
        .collect()
}

fn run_single_gate(project_root: &Path, gate: &QualityGate) -> GateResult {
    let start = Instant::now();
    let result = std::process::Command::new(&gate.command)
        .args(&gate.args)
        .current_dir(project_root)
        .output();
    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            GateResult {
                gate_name: gate.name.clone(),
                passed: output.status.success(),
                exit_code,
                stdout: truncate_tail(&String::from_utf8_lossy(&output.stdout), 2000),
                stderr: truncate_tail(&String::from_utf8_lossy(&output.stderr), 2000),
                duration_ms,
            }
        }
        Err(e) => GateResult {
            gate_name: gate.name.clone(),
            passed: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("Failed to run: {e}"),
            duration_ms,
        },
    }
}

/// Produce a human-readable summary of all failed gates.
pub fn summarize_gate_failures(results: &[GateResult]) -> String {
    results
        .iter()
        .filter(|r| !r.passed)
        .map(|r| {
            let stderr_tail = truncate_tail(&r.stderr, 500);
            format!(
                "FAILED: {} (exit {})\n{}\n---",
                r.gate_name, r.exit_code, stderr_tail
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_gates_node_project() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest","lint":"eslint ."}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();

        let gates = detect_gates(dir.path());
        let names: Vec<&str> = gates.iter().map(|g| g.name.as_str()).collect();
        assert!(
            names.contains(&"TypeCheck"),
            "expected TypeCheck, got {names:?}"
        );
        assert!(
            names.contains(&"Test Suite"),
            "expected Test Suite, got {names:?}"
        );
        assert!(names.contains(&"Lint"), "expected Lint, got {names:?}");
        assert_eq!(gates.len(), 3);
    }

    #[test]
    fn test_detect_gates_node_with_typecheck_script() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest","typecheck":"tsc --noEmit"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();

        let gates = detect_gates(dir.path());
        let tc = gates.iter().find(|g| g.name == "TypeCheck").unwrap();
        // Should use npm run typecheck, not npx tsc
        assert_eq!(tc.command, "npm");
        assert_eq!(tc.args, vec!["run", "typecheck"]);
    }

    #[test]
    fn test_detect_gates_rust_project() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let gates = detect_gates(dir.path());
        assert_eq!(gates.len(), 2);
        assert_eq!(gates[0].name, "Cargo Test");
        assert_eq!(gates[1].name, "Cargo Clippy");
    }

    #[test]
    fn test_detect_gates_empty_project() {
        let dir = tempdir().unwrap();
        let gates = detect_gates(dir.path());
        assert!(gates.is_empty());
    }

    #[test]
    fn test_detect_gates_python_project() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "").unwrap();

        let gates = detect_gates(dir.path());
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].name, "Pytest");
    }

    #[test]
    fn test_detect_gates_playwright() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("playwright.config.ts"), "export default {}").unwrap();

        let gates = detect_gates(dir.path());
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].name, "Playwright E2E");
        assert_eq!(gates[0].command, "npx");
        assert_eq!(gates[0].args, vec!["playwright", "test", "--reporter=list"]);
    }

    #[test]
    fn test_detect_gates_playwright_js() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("playwright.config.js"),
            "module.exports = {}",
        )
        .unwrap();

        let gates = detect_gates(dir.path());
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].name, "Playwright E2E");
    }

    #[test]
    fn test_run_gates_success() {
        let dir = tempdir().unwrap();
        let gate = QualityGate {
            name: "Echo".into(),
            command: "echo".into(),
            args: vec!["hello".into()],
        };
        let results = run_gates(dir.path(), &[gate]);
        assert_eq!(results.len(), 1);
        assert!(results[0].passed);
        assert_eq!(results[0].exit_code, 0);
        assert!(results[0].stdout.contains("hello"));
    }

    #[test]
    fn test_run_gates_failure() {
        let dir = tempdir().unwrap();
        let gate = QualityGate {
            name: "Fail".into(),
            command: "false".into(),
            args: vec![],
        };
        let results = run_gates(dir.path(), &[gate]);
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        assert_ne!(results[0].exit_code, 0);
    }

    #[test]
    fn test_run_gates_missing_command() {
        let dir = tempdir().unwrap();
        let gate = QualityGate {
            name: "Missing".into(),
            command: "nonexistent_binary_xyz_42".into(),
            args: vec![],
        };
        let results = run_gates(dir.path(), &[gate]);
        assert_eq!(results.len(), 1);
        assert!(!results[0].passed);
        assert_eq!(results[0].exit_code, -1);
        assert!(results[0].stderr.contains("Failed to run"));
    }

    #[test]
    fn test_truncate_tail_short() {
        let s = "short";
        assert_eq!(truncate_tail(s, 100), "short");
    }

    #[test]
    fn test_truncate_tail_long() {
        let s = "a".repeat(3000);
        let result = truncate_tail(&s, 2000);
        assert!(result.starts_with("...truncated..."));
        assert!(result.len() <= 2020); // 2000 + marker
    }

    #[test]
    fn test_summarize_failures() {
        let results = vec![
            GateResult {
                gate_name: "TypeCheck".into(),
                passed: false,
                exit_code: 2,
                stdout: String::new(),
                stderr: "TS2345: type error".into(),
                duration_ms: 100,
            },
            GateResult {
                gate_name: "Lint".into(),
                passed: true,
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 50,
            },
            GateResult {
                gate_name: "Tests".into(),
                passed: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: "FAIL src/foo.test.ts".into(),
                duration_ms: 200,
            },
        ];
        let summary = summarize_gate_failures(&results);
        assert!(summary.contains("FAILED: TypeCheck"));
        assert!(summary.contains("FAILED: Tests"));
        assert!(!summary.contains("Lint")); // passed — should be excluded
    }
}
