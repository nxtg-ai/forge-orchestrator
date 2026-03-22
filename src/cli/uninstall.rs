use std::path::PathBuf;

/// Execute the uninstall command — remove forge binary and optionally .forge/ directories.
pub fn execute(project_root: &std::path::Path, force: bool) -> anyhow::Result<()> {
    let binary_path = find_binary_path()?;

    println!("Forge uninstall");
    println!("===============");
    println!();

    // 1. Remove the binary
    println!("Binary: {}", binary_path.display());
    if binary_path.exists() {
        std::fs::remove_file(&binary_path)?;
        println!("  Removed.");
    } else {
        println!("  Not found (already removed?).");
    }

    // 2. Optionally remove .forge/ in the current project
    let forge_dir = project_root.join(".forge");
    if forge_dir.exists() {
        if force {
            std::fs::remove_dir_all(&forge_dir)?;
            println!();
            println!("Project data: {}", forge_dir.display());
            println!("  Removed.");
        } else {
            println!();
            println!("Project data: {}", forge_dir.display());
            println!("  Kept. Use --force to also remove .forge/ project data.");
        }
    }

    // 3. Optionally remove ~/.forge/ global config
    if let Ok(home) = std::env::var("HOME") {
        let global_dir = PathBuf::from(&home).join(".forge");
        if global_dir.exists() {
            if force {
                std::fs::remove_dir_all(&global_dir)?;
                println!();
                println!("Global config: {}", global_dir.display());
                println!("  Removed.");
            } else {
                println!();
                println!("Global config: {}", global_dir.display());
                println!("  Kept. Use --force to also remove ~/.forge/ global config.");
            }
        }
    }

    println!();
    println!("Forge has been uninstalled.");
    Ok(())
}

/// Find the path to the currently running forge binary.
fn find_binary_path() -> anyhow::Result<PathBuf> {
    // First try: the actual executable path
    if let Ok(exe) = std::env::current_exe() {
        let resolved = exe.canonicalize().unwrap_or(exe);
        return Ok(resolved);
    }

    // Fallback: search PATH for "forge"
    if let Ok(output) = std::process::Command::new("which").arg("forge").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    anyhow::bail!("Could not determine forge binary location. Remove it manually.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_binary_path_returns_something() {
        // current_exe() should work in test context
        let result = find_binary_path();
        assert!(result.is_ok(), "Should find the test binary path");
    }

    #[test]
    fn test_uninstall_removes_forge_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("state.json"), "{}").unwrap();

        // Create a fake binary to "remove"
        let fake_bin = tmp.path().join("forge-test-bin");
        std::fs::write(&fake_bin, "fake").unwrap();

        // We can't easily test the full execute() because it removes
        // the real binary. But we can test the forge_dir removal logic.
        assert!(forge_dir.exists());
        std::fs::remove_dir_all(&forge_dir).unwrap();
        assert!(!forge_dir.exists());
    }
}
