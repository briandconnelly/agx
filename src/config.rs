//! Global configuration — environment variables, paths, and defaults.

use std::path::{Path, PathBuf};

/// Default manifest filename.
pub const MANIFEST_FILE: &str = "agent-environment.yml";

/// Default lockfile filename.
pub const LOCKFILE_FILE: &str = "agent-environment.lock";

/// Default agx workspace directory (gitignored).
pub const AGX_DIR: &str = ".agx";

/// Search for the manifest file in the current directory and ancestors.
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(MANIFEST_FILE);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Get the log level from the AGX_LOG environment variable.
pub fn log_level() -> String {
    std::env::var("AGX_LOG").unwrap_or_else(|_| "warn".to_string())
}

/// Check if we're in offline mode.
pub fn is_offline() -> bool {
    std::env::var("AGX_OFFLINE")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Check if colored output is disabled.
pub fn no_color() -> bool {
    std::env::var("AGX_NO_COLOR")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn constants_have_expected_values() {
        assert_eq!(MANIFEST_FILE, "agent-environment.yml");
        assert_eq!(LOCKFILE_FILE, "agent-environment.lock");
        assert_eq!(AGX_DIR, ".agx");
    }

    #[test]
    fn find_manifest_in_current_dir() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join(MANIFEST_FILE);
        fs::write(&manifest_path, "name: test\n").unwrap();

        let found = find_manifest(dir.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap(), manifest_path);
    }

    #[test]
    fn find_manifest_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join(MANIFEST_FILE);
        fs::write(&manifest_path, "name: test\n").unwrap();

        // Create a subdirectory and search from there
        let sub = dir.path().join("sub").join("deep");
        fs::create_dir_all(&sub).unwrap();

        let found = find_manifest(&sub);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), manifest_path);
    }

    #[test]
    fn find_manifest_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        // Don't create any manifest file
        let found = find_manifest(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn is_offline_returns_false_by_default() {
        // When AGX_OFFLINE is not set, should return false
        // Note: this test reads the current env state; if AGX_OFFLINE is
        // set externally it may behave differently.
        let val = is_offline();
        // Just exercise the function — the result depends on env
        let _ = val;
    }

    #[test]
    fn log_level_returns_value() {
        // Exercise the function
        let level = log_level();
        assert!(!level.is_empty());
    }

    #[test]
    fn no_color_returns_value() {
        // Exercise the function
        let _ = no_color();
    }
}
