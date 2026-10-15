//! Helpers shared by the integration tests. Every test that runs `git`, or runs `forge` (which
//! runs git itself), builds the command here, so none of them inherits the parent's git
//! environment. `git_env_guard.rs` fails if a test file spawns git any other way.
#![allow(dead_code)] // each test crate uses a different subset

use std::process::Command;

/// Variables git reads before it looks at the working directory. A test run from a git hook
/// inherits them (a pre-push hook in a linked worktree exports an absolute `GIT_DIR`), and then
/// a fixture's `git init`, `config`, `add` and `commit` write to that outer repo instead of the
/// fixture.
pub const GIT_ENV: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// Clear [`GIT_ENV`] from a command about to run in a fixture.
pub fn clear_git_env(cmd: &mut Command) -> &mut Command {
    for var in GIT_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// A `git` command that cannot reach a repo named by the parent's environment.
pub fn git() -> Command {
    let mut cmd = Command::new("git");
    clear_git_env(&mut cmd);
    cmd
}

/// The built `forge` binary, under the same rule (it runs git for context and commits).
#[allow(deprecated)] // cargo_bin deprecation: tracked, not blocking
pub fn forge() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("forge").unwrap();
    for var in GIT_ENV {
        cmd.env_remove(var);
    }
    cmd
}
