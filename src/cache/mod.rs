//! Global cache management — XDG-compliant, content-addressed storage.
//!
//! Cache layout:
//! - `$CACHE/plugins/{name}/{version}/` — plugin contents
//! - `$CACHE/skills/{name}/{commit}/` — skill contents
//! - `$CACHE/agents/{name}/{commit}/` — agent file
//! - `$CACHE/git/{url-slug}.git` — bare repos (managed by git client)

pub mod integrity;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A single entry in the global cache.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// Category: "plugins", "skills", "agents", or "git".
    pub category: String,
    /// Asset name (or repo slug for git).
    pub name: String,
    /// Version string (semver for plugins) or commit hash.
    pub version_or_commit: String,
    /// Total size in bytes.
    pub size_bytes: u64,
    /// Absolute path to the cached entry.
    pub path: PathBuf,
}

/// Get the global cache directory path.
///
/// Priority: `AGX_CACHE_DIR` env var > XDG cache dir > platform default.
pub fn cache_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("AGX_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = dirs::cache_dir().context("could not determine cache directory")?;
    Ok(base.join("agx"))
}

/// Subdirectory within the cache for plugins.
pub fn plugins_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("plugins"))
}

/// Subdirectory within the cache for skills.
pub fn skills_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("skills"))
}

/// Subdirectory within the cache for agents.
pub fn agents_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("agents"))
}

/// Subdirectory within the cache for bare git repos.
pub fn git_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("git"))
}

/// Subdirectory within the cache for registry indexes.
pub fn registry_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("registry"))
}

/// Store a downloaded plugin in the cache. Returns the cache path.
///
/// Copies the contents of `source_dir` into `$CACHE/plugins/{name}/{version}/`.
pub fn store_plugin(name: &str, version: &str, source_dir: &Path) -> Result<PathBuf> {
    let dest = plugins_dir()?.join(name).join(version);
    copy_dir_to(source_dir, &dest)
        .with_context(|| format!("caching plugin {name}@{version}"))?;
    Ok(dest)
}

/// Store a downloaded skill in the cache. Returns the cache path.
///
/// Copies the contents of `source_dir` into `$CACHE/skills/{name}/{commit}/`.
pub fn store_skill(name: &str, commit: &str, source_dir: &Path) -> Result<PathBuf> {
    let dest = skills_dir()?.join(name).join(commit);
    copy_dir_to(source_dir, &dest)
        .with_context(|| format!("caching skill {name}@{commit}"))?;
    Ok(dest)
}

/// Store a downloaded agent in the cache. Returns the cache path.
///
/// Copies the file at `source_path` into `$CACHE/agents/{name}/{commit}/`.
pub fn store_agent(name: &str, commit: &str, source_path: &Path) -> Result<PathBuf> {
    let dest_dir = agents_dir()?.join(name).join(commit);
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("creating cache dir {}", dest_dir.display()))?;

    let file_name = source_path
        .file_name()
        .context("source_path has no filename")?;
    let dest = dest_dir.join(file_name);
    std::fs::copy(source_path, &dest)
        .with_context(|| format!("caching agent {name}@{commit}"))?;
    Ok(dest_dir)
}

/// Look up a cached plugin. Returns `Some(path)` if the cache entry exists.
pub fn get_plugin(name: &str, version: &str) -> Result<Option<PathBuf>> {
    let path = plugins_dir()?.join(name).join(version);
    Ok(if path.exists() { Some(path) } else { None })
}

/// Look up a cached skill. Returns `Some(path)` if the cache entry exists.
pub fn get_skill(name: &str, commit: &str) -> Result<Option<PathBuf>> {
    let path = skills_dir()?.join(name).join(commit);
    Ok(if path.exists() { Some(path) } else { None })
}

/// Look up a cached agent. Returns `Some(path)` if the cache entry exists.
pub fn get_agent(name: &str, commit: &str) -> Result<Option<PathBuf>> {
    let path = agents_dir()?.join(name).join(commit);
    Ok(if path.exists() { Some(path) } else { None })
}

/// Remove all cached data.
pub fn clean_all() -> Result<()> {
    let dir = cache_dir()?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("removing {}", dir.display()))?;
    }
    Ok(())
}

/// Clean cache entries for a specific asset name.
///
/// Searches all categories (plugins, skills, agents) and removes any
/// directory matching the given name.
pub fn clean_asset(name: &str) -> Result<()> {
    let mut found = false;
    for category in &["plugins", "skills", "agents"] {
        let path = cache_dir()?.join(category).join(name);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}/{name}", category))?;
            found = true;
        }
    }
    if !found {
        anyhow::bail!("no cached entries found for '{name}'");
    }
    Ok(())
}

/// List all cached assets with detailed information.
pub fn list_cached() -> Result<Vec<CacheEntry>> {
    let dir = cache_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();

    // Scan versioned categories: plugins/{name}/{version}, skills/{name}/{commit}, agents/{name}/{commit}
    for category in &["plugins", "skills", "agents"] {
        let cat_dir = dir.join(category);
        if !cat_dir.exists() {
            continue;
        }
        if let Ok(names) = std::fs::read_dir(&cat_dir) {
            for name_entry in names.flatten() {
                if !name_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = name_entry.file_name().to_string_lossy().to_string();
                if let Ok(versions) = std::fs::read_dir(name_entry.path()) {
                    for ver_entry in versions.flatten() {
                        if !ver_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            continue;
                        }
                        let version = ver_entry.file_name().to_string_lossy().to_string();
                        let size = dir_size(&ver_entry.path())?;
                        entries.push(CacheEntry {
                            category: category.to_string(),
                            name: name.clone(),
                            version_or_commit: version,
                            size_bytes: size,
                            path: ver_entry.path(),
                        });
                    }
                }
            }
        }
    }

    // Scan git bare repos: git/{slug}.git
    let git_path = dir.join("git");
    if git_path.exists() {
        if let Ok(repos) = std::fs::read_dir(&git_path) {
            for entry in repos.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let size = dir_size(&entry.path())?;
                entries.push(CacheEntry {
                    category: "git".to_string(),
                    name: name.clone(),
                    version_or_commit: String::new(),
                    size_bytes: size,
                    path: entry.path(),
                });
            }
        }
    }

    Ok(entries)
}

/// Format a byte count as a human-readable string (e.g. "1.2 MB").
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Recursively copy a directory tree from `src` to `dest`.
fn copy_dir_to(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("clearing existing cache at {}", dest.display()))?;
    }
    std::fs::create_dir_all(dest)
        .with_context(|| format!("creating {}", dest.display()))?;

    for entry in walkdir::WalkDir::new(src) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(src)?;
        let target = dest.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Global lock to serialize tests that mutate `AGX_CACHE_DIR`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: set AGX_CACHE_DIR to a temp directory for test isolation.
    /// Holds a process-wide lock to prevent parallel tests from interfering.
    fn with_temp_cache(f: impl FnOnce(&Path)) {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // Safety: we hold ENV_LOCK so no other test is reading this var.
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };
        f(tmp.path());
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    #[test]
    fn store_and_retrieve_plugin() {
        with_temp_cache(|_cache| {
            let src = tempfile::tempdir().unwrap();
            fs::write(src.path().join("plugin.yml"), "name: test-plugin").unwrap();
            fs::create_dir_all(src.path().join("skills")).unwrap();
            fs::write(src.path().join("skills/review.md"), "# Review").unwrap();

            let cached = store_plugin("test-plugin", "1.2.0", src.path()).unwrap();
            assert!(cached.exists());
            assert!(cached.join("plugin.yml").exists());
            assert!(cached.join("skills/review.md").exists());

            // Retrieve
            let found = get_plugin("test-plugin", "1.2.0").unwrap();
            assert_eq!(found, Some(cached));

            // Miss
            let miss = get_plugin("test-plugin", "2.0.0").unwrap();
            assert!(miss.is_none());
        });
    }

    #[test]
    fn store_and_retrieve_skill() {
        with_temp_cache(|_cache| {
            let src = tempfile::tempdir().unwrap();
            fs::write(src.path().join("skill.md"), "# Deploy").unwrap();

            let cached = store_skill("deploy", "abc123", src.path()).unwrap();
            assert!(cached.join("skill.md").exists());

            let found = get_skill("deploy", "abc123").unwrap();
            assert_eq!(found, Some(cached));
        });
    }

    #[test]
    fn store_and_retrieve_agent() {
        with_temp_cache(|_cache| {
            let src = tempfile::tempdir().unwrap();
            let agent_file = src.path().join("reviewer.md");
            fs::write(&agent_file, "# Reviewer agent").unwrap();

            let cached = store_agent("reviewer", "def456", &agent_file).unwrap();
            assert!(cached.join("reviewer.md").exists());

            let found = get_agent("reviewer", "def456").unwrap();
            assert_eq!(found, Some(cached));
        });
    }

    #[test]
    fn two_versions_stored_separately() {
        with_temp_cache(|_cache| {
            let src1 = tempfile::tempdir().unwrap();
            fs::write(src1.path().join("data"), "v1").unwrap();
            let src2 = tempfile::tempdir().unwrap();
            fs::write(src2.path().join("data"), "v2").unwrap();

            let p1 = store_plugin("myplugin", "1.0.0", src1.path()).unwrap();
            let p2 = store_plugin("myplugin", "2.0.0", src2.path()).unwrap();

            assert_ne!(p1, p2);
            assert_eq!(fs::read_to_string(p1.join("data")).unwrap(), "v1");
            assert_eq!(fs::read_to_string(p2.join("data")).unwrap(), "v2");
        });
    }

    #[test]
    fn clean_asset_removes_matching() {
        with_temp_cache(|_cache| {
            let src = tempfile::tempdir().unwrap();
            fs::write(src.path().join("data"), "test").unwrap();

            store_plugin("to-remove", "1.0.0", src.path()).unwrap();
            store_skill("to-remove", "abc", src.path()).unwrap();

            // Should exist before clean
            assert!(get_plugin("to-remove", "1.0.0").unwrap().is_some());
            assert!(get_skill("to-remove", "abc").unwrap().is_some());

            clean_asset("to-remove").unwrap();

            assert!(get_plugin("to-remove", "1.0.0").unwrap().is_none());
            assert!(get_skill("to-remove", "abc").unwrap().is_none());
        });
    }

    #[test]
    fn clean_asset_errors_on_missing() {
        with_temp_cache(|_cache| {
            let result = clean_asset("nonexistent");
            assert!(result.is_err());
        });
    }

    #[test]
    fn clean_all_removes_everything() {
        with_temp_cache(|cache| {
            let src = tempfile::tempdir().unwrap();
            fs::write(src.path().join("data"), "test").unwrap();

            store_plugin("p", "1.0.0", src.path()).unwrap();
            store_skill("s", "abc", src.path()).unwrap();

            assert!(cache.exists());
            clean_all().unwrap();
            assert!(!cache.join("plugins").exists());
            assert!(!cache.join("skills").exists());
        });
    }

    #[test]
    fn list_cached_returns_entries() {
        with_temp_cache(|_cache| {
            let src = tempfile::tempdir().unwrap();
            fs::write(src.path().join("data"), "hello").unwrap();

            store_plugin("my-plugin", "1.0.0", src.path()).unwrap();
            store_skill("my-skill", "abc123", src.path()).unwrap();

            let entries = list_cached().unwrap();
            assert!(entries.len() >= 2);

            let plugin_entry = entries.iter().find(|e| e.category == "plugins" && e.name == "my-plugin");
            assert!(plugin_entry.is_some());
            let pe = plugin_entry.unwrap();
            assert_eq!(pe.version_or_commit, "1.0.0");
            assert!(pe.size_bytes > 0);

            let skill_entry = entries.iter().find(|e| e.category == "skills" && e.name == "my-skill");
            assert!(skill_entry.is_some());
        });
    }

    #[test]
    fn list_cached_empty_returns_empty() {
        with_temp_cache(|_cache| {
            let entries = list_cached().unwrap();
            assert!(entries.is_empty());
        });
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }
}
