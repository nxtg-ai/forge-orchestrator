//! Every test that spawns git must go through `common::git()`, which clears the inherited git
//! environment. A direct `Command::new("git")` in a test file is the bug this guards: under a
//! pre-push hook in a linked worktree it wrote fixture commits and config into the pushed repo.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read test dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn tests_spawn_git_only_through_the_shared_helper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("tests"), &mut files);
    // The helper itself, and this guard (whose doc comment names the pattern).
    let allowed = [
        root.join("tests/common/mod.rs"),
        root.join("tests/git_env_guard.rs"),
    ];
    let mut offenders = Vec::new();
    for path in files.iter().filter(|p| !allowed.contains(p)) {
        let text = fs::read_to_string(path).expect("read test file");
        for (n, line) in text.lines().enumerate() {
            if line.contains(concat!("Command::new(", "\"git\")")) {
                let rel = path.strip_prefix(root).unwrap().display();
                offenders.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    assert!(
        files.len() > allowed.len(),
        "found no test files to check under {}",
        root.join("tests").display()
    );
    assert!(
        offenders.is_empty(),
        "spawn git through common::git() so the inherited GIT_DIR is cleared:\n{}",
        offenders.join("\n")
    );
}
