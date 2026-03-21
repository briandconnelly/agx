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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // resolve
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_existing_dir() {
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("skills").join("deploy");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("skill.md"), "# Deploy").unwrap();

        let result = resolve(root.path(), "./skills/deploy").unwrap();
        assert_eq!(result, skill_dir);
    }

    #[test]
    fn resolve_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let agents_dir = root.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let agent_file = agents_dir.join("reviewer.md");
        fs::write(&agent_file, "# Reviewer").unwrap();

        let result = resolve(root.path(), "./agents/reviewer.md").unwrap();
        assert_eq!(result, agent_file);
    }

    #[test]
    fn resolve_without_dot_slash_prefix() {
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("skills").join("lint");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("skill.md"), "# Lint").unwrap();

        let result = resolve(root.path(), "skills/lint").unwrap();
        assert_eq!(result, skill_dir);
    }

    #[test]
    fn resolve_nonexistent_path_errors() {
        let root = tempfile::tempdir().unwrap();

        let result = resolve(root.path(), "./nonexistent/path");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("local asset not found"));
    }

    #[test]
    fn resolve_strips_dot_slash() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("my-file.txt");
        fs::write(&file, "content").unwrap();

        // With ./ prefix
        let result1 = resolve(root.path(), "./my-file.txt").unwrap();
        // Without ./ prefix
        let result2 = resolve(root.path(), "my-file.txt").unwrap();
        assert_eq!(result1, result2);
    }

    // -----------------------------------------------------------------------
    // copy_asset
    // -----------------------------------------------------------------------

    #[test]
    fn copy_asset_file() {
        let src_dir = tempfile::tempdir().unwrap();
        let src_file = src_dir.path().join("source.txt");
        fs::write(&src_file, "file content").unwrap();

        let dest_dir = tempfile::tempdir().unwrap();
        let dest_file = dest_dir.path().join("nested").join("dest.txt");

        copy_asset(&src_file, &dest_file).unwrap();

        assert!(dest_file.exists());
        assert_eq!(fs::read_to_string(&dest_file).unwrap(), "file content");
    }

    #[test]
    fn copy_asset_directory() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("a.txt"), "alpha").unwrap();
        let sub = src.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("b.txt"), "beta").unwrap();

        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("copied");

        copy_asset(src.path(), &dest).unwrap();

        assert!(dest.join("a.txt").exists());
        assert!(dest.join("sub").join("b.txt").exists());
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
        assert_eq!(
            fs::read_to_string(dest.join("sub").join("b.txt")).unwrap(),
            "beta"
        );
    }

    #[test]
    fn copy_asset_creates_parent_dirs() {
        let src_dir = tempfile::tempdir().unwrap();
        let src_file = src_dir.path().join("data.bin");
        fs::write(&src_file, b"binary data").unwrap();

        let dest_dir = tempfile::tempdir().unwrap();
        let dest_file = dest_dir.path().join("a").join("b").join("c").join("data.bin");

        copy_asset(&src_file, &dest_file).unwrap();
        assert!(dest_file.exists());
        assert_eq!(fs::read(&dest_file).unwrap(), b"binary data");
    }
}
