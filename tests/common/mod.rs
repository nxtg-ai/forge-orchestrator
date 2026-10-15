//! Helpers shared by the integration tests. Every test that runs `git`, or runs `forge` (which
//! runs git itself), builds the command here, so none of them inherits the parent's git
//! environment. `git_env_guard.rs` fails if a test file spawns git any other way.
//!
//! Every spawned `forge` also gets a test HOME ([`test_home`]) instead of the caller's. `forge
//! sync` renders the agy adapter config, which adds the project to
//! `$HOME/.gemini/antigravity-cli/settings.json`; with the real HOME, every test project landed
//! in the developer's own agy config. `forge init` and `sync` also run `agy --version` to detect
//! tools, so a stub `agy` comes first on PATH and the real agy is never started, and `BROWSER`
//! is `/bin/false`. `home_guard.rs` fails if a test file spawns forge any other way.
#![allow(dead_code)] // each test crate uses a different subset

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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

/// The HOME every spawned `forge` runs with: `<target>/tmp/test-home`, never the caller's. Its
/// `bin/` holds the stub `agy` that [`test_path`] puts first on PATH.
pub fn test_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-home");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).expect("create test HOME");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Written to a per-process name, then renamed, so parallel test binaries never see a
            // half-written stub.
            let tmp = bin.join(format!(".agy.{}", std::process::id()));
            std::fs::write(&tmp, "#!/bin/sh\necho 0.0.0-forge-test-stub\n")
                .expect("write agy stub");
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .expect("chmod agy stub");
            std::fs::rename(&tmp, bin.join("agy")).expect("install agy stub");
        }
        home
    })
}

/// PATH for a spawned `forge`: the stub `agy` directory first, then the caller's PATH.
pub fn test_path() -> std::ffi::OsString {
    let mut dirs = vec![test_home().join("bin")];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(dirs).expect("join PATH")
}

/// The environment every spawned `forge` gets: a test HOME, the stub `agy` first on PATH, and
/// no browser opener.
pub fn test_env() -> Vec<(&'static str, std::ffi::OsString)> {
    vec![
        ("HOME", test_home().as_os_str().to_owned()),
        ("USERPROFILE", test_home().as_os_str().to_owned()),
        ("PATH", test_path()),
        ("BROWSER", "/bin/false".into()),
    ]
}

/// The built `forge` binary, under the same rule (it runs git for context and commits), with
/// [`test_env`].
#[allow(deprecated)] // cargo_bin deprecation: tracked, not blocking
pub fn forge() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("forge").unwrap();
    for var in GIT_ENV {
        cmd.env_remove(var);
    }
    cmd.envs(test_env());
    cmd
}

/// [`forge`] as a `std::process::Command`, for tests that drive stdin/stdout themselves.
pub fn forge_std() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    clear_git_env(&mut cmd);
    cmd.envs(test_env());
    cmd
}
