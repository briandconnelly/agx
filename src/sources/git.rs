//! Git source client — clone, fetch, resolve refs to SHAs, extract files.
//!
//! For the MVP, this shells out to `git` rather than using libgit2,
//! to avoid build complexity.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Clone a git repository as a bare repo to a destination path.
///
/// Sets `current_dir` to the destination's parent so the command does not
/// depend on the process working directory being valid.
pub fn clone(url: &str, dest: &Path) -> Result<()> {
    let parent = dest
        .parent()
        .context("destination path has no parent directory")?;
    let status = Command::new("git")
        .args(["clone", "--bare", url])
        .arg(dest)
        .current_dir(parent)
        .status()
        .context("running git clone")?;
    if !status.success() {
        anyhow::bail!("git clone failed for {url}");
    }
    Ok(())
}

/// Fetch all updates for an existing bare repository.
///
/// Explicitly fetches branches and tags from origin into the local refs,
/// since bare clones don't set a default fetch refspec.
pub fn fetch(repo_path: &Path) -> Result<()> {
    // Fetch branches into local branch refs
    let status = Command::new("git")
        .args([
            "fetch",
            "origin",
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
            "--prune",
        ])
        .current_dir(repo_path)
        .status()
        .context("running git fetch")?;
    if !status.success() {
        anyhow::bail!("git fetch failed in {}", repo_path.display());
    }
    Ok(())
}

/// Resolve a ref (tag, branch, or commit prefix) to a full SHA in a bare repo.
pub fn resolve_ref(repo_path: &Path, reference: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(repo_path)
        .output()
        .context("running git rev-parse")?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse failed for ref '{}' in {}",
            reference,
            repo_path.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve the default branch (HEAD) to a full SHA in a bare repo.
///
/// This is used when no explicit ref is specified in a dependency.
pub fn resolve_default_branch(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .context("running git rev-parse HEAD")?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse HEAD failed in {}",
            repo_path.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Extract a file or directory from a bare repo at a given commit.
///
/// Uses `git archive` piped to `tar` to extract the specified `path`
/// from the repo at `commit` into the `dest` directory.
pub fn extract(repo_path: &Path, commit: &str, path: &str, dest: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["archive", commit, "--", path])
        .current_dir(repo_path)
        .output()
        .context("running git archive")?;
    if !output.status.success() {
        anyhow::bail!(
            "git archive failed for '{}' at {} in {}",
            path,
            commit,
            repo_path.display()
        );
    }

    std::fs::create_dir_all(dest)
        .with_context(|| format!("creating destination directory {}", dest.display()))?;

    let mut child = Command::new("tar")
        .args(["xf", "-"])
        .current_dir(dest)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("spawning tar")?;

    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("tar stdin should be piped")
            .write_all(&output.stdout)
            .context("writing archive data to tar")?;
    }

    let status = child.wait().context("waiting for tar")?;
    if !status.success() {
        anyhow::bail!("tar extraction failed");
    }
    Ok(())
}

/// Convert a git URL to a cache-safe directory name.
///
/// Strips the scheme, replaces `/` with `-`, and appends `.git`.
///
/// # Examples
///
/// ```
/// use agx::sources::git::url_to_slug;
/// assert_eq!(url_to_slug("https://github.com/org/repo"), "github.com-org-repo.git");
/// assert_eq!(url_to_slug("git@github.com:org/repo.git"), "github.com-org-repo.git");
/// ```
pub fn url_to_slug(url: &str) -> String {
    let stripped = url
        // Remove common scheme prefixes
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("git://")
        // Handle SSH-style URLs: git@host:path → host/path
        .trim_start_matches("git@");

    // Normalize SSH colon syntax (git@host:org/repo) into slash syntax
    let normalized = stripped.replacen(':', "/", 1);

    // Strip trailing .git if present (we'll add it back)
    let without_suffix = normalized
        .trim_end_matches('/')
        .trim_end_matches(".git");

    // Replace slashes with dashes
    let slug = without_suffix.replace('/', "-");

    format!("{slug}.git")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_to_slug_https() {
        assert_eq!(
            url_to_slug("https://github.com/org/repo"),
            "github.com-org-repo.git"
        );
    }

    #[test]
    fn test_url_to_slug_https_with_git_suffix() {
        assert_eq!(
            url_to_slug("https://github.com/org/repo.git"),
            "github.com-org-repo.git"
        );
    }

    #[test]
    fn test_url_to_slug_ssh() {
        assert_eq!(
            url_to_slug("git@github.com:org/repo.git"),
            "github.com-org-repo.git"
        );
    }

    #[test]
    fn test_url_to_slug_git_protocol() {
        assert_eq!(
            url_to_slug("git://example.com/some/repo"),
            "example.com-some-repo.git"
        );
    }

    #[test]
    fn test_url_to_slug_trailing_slash() {
        assert_eq!(
            url_to_slug("https://github.com/org/repo/"),
            "github.com-org-repo.git"
        );
    }
}
