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
