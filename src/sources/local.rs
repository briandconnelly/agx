//! Local path source — resolve and validate local asset paths.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve a local path relative to the project root, and verify it exists.
pub fn resolve(project_root: &Path, relative_path: &str) -> Result<PathBuf> {
    let resolved = project_root.join(
        relative_path
            .strip_prefix("./")
            .unwrap_or(relative_path),
    );
    if !resolved.exists() {
        anyhow::bail!("local asset not found: {}", resolved.display());
    }
    Ok(resolved)
}

/// Copy a local asset (file or directory) to a destination.
pub fn copy_asset(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        crate::backends::copy_dir_recursive(src, dest)?;
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(src, dest)
            .with_context(|| format!("copying {} to {}", src.display(), dest.display()))?;
    }
    Ok(())
}
