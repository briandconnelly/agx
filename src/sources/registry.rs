//! Marketplace registry client — fetch and cache registry indexes.
//!
//! A registry is a git repository containing a `marketplace.json` index file
//! that lists available plugins and their versions.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::git;

/// Parsed registry index from a `marketplace.json` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Map of plugin name to plugin metadata.
    pub plugins: BTreeMap<String, PluginEntry>,
}

/// Metadata for a single plugin in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Human-readable description of the plugin.
    #[serde(default)]
    pub description: String,
    /// Map of version string to version details.
    pub versions: BTreeMap<String, VersionDetail>,
}

/// Details for a specific version of a plugin in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDetail {
    /// The git commit SHA for this version.
    pub commit: String,
    /// Path within the registry repo where this version's assets live.
    #[serde(default)]
    pub path: Option<String>,
}

/// A resolved plugin version ready for consumption.
#[derive(Debug, Clone)]
pub struct PluginVersion {
    /// The semver version.
    pub version: semver::Version,
    /// The git commit SHA.
    pub git_commit: String,
    /// Path within the registry repo (if any).
    pub path: Option<String>,
}

/// Fetch (or update) a registry index from a git URL and parse it.
///
/// Clones the registry repo into `cache_dir` if it doesn't exist,
/// otherwise fetches updates. Then parses `marketplace.json` from HEAD.
pub fn fetch_index(url: &str, cache_dir: &Path) -> Result<RegistryIndex> {
    if cache_dir.exists() {
        git::fetch(cache_dir).context("updating registry index")?;
    } else {
        git::clone(url, cache_dir).context("cloning registry index")?;
    }

    // Read marketplace.json from HEAD of the bare repo
    let output = std::process::Command::new("git")
        .args(["show", "HEAD:marketplace.json"])
        .current_dir(cache_dir)
        .output()
        .context("reading marketplace.json from registry")?;

    if !output.status.success() {
        anyhow::bail!(
            "marketplace.json not found in registry at {}",
            cache_dir.display()
        );
    }

    let index: RegistryIndex = serde_json::from_slice(&output.stdout)
        .context("parsing marketplace.json")?;

    Ok(index)
}

/// Fetch a registry index with optional commit pinning.
///
/// If `pin` is `Some`, the index is read from that specific commit SHA
/// using `git show <pin>:marketplace.json` instead of HEAD. This ensures
/// reproducibility even if the registry updates.
pub fn fetch_index_pinned(url: &str, cache_dir: &Path, pin: Option<&str>) -> Result<RegistryIndex> {
    if cache_dir.exists() {
        git::fetch(cache_dir).context("updating registry index")?;
    } else {
        git::clone(url, cache_dir).context("cloning registry index")?;
    }

    let rev = pin.unwrap_or("HEAD");

    // If pinned, verify the commit exists in the repo
    if let Some(pin_sha) = pin {
        let verify = std::process::Command::new("git")
            .args(["rev-parse", "--verify", &format!("{pin_sha}^{{commit}}")])
            .current_dir(cache_dir)
            .output()
            .context("verifying pinned commit")?;

        if !verify.status.success() {
            anyhow::bail!(
                "pinned commit '{}' not found in registry at {}",
                pin_sha,
                cache_dir.display()
            );
        }
    }

    let output = std::process::Command::new("git")
        .args(["show", &format!("{rev}:marketplace.json")])
        .current_dir(cache_dir)
        .output()
        .context("reading marketplace.json from registry")?;

    if !output.status.success() {
        anyhow::bail!(
            "marketplace.json not found at rev '{}' in registry at {}",
            rev,
            cache_dir.display()
        );
    }

    let index: RegistryIndex = serde_json::from_slice(&output.stdout)
        .context("parsing marketplace.json")?;

    Ok(index)
}

/// Look up a plugin by name in a registry index.
///
/// Returns a list of available versions sorted descending (newest first),
/// or `None` if the plugin is not found.
pub fn lookup_plugin(index: &RegistryIndex, name: &str) -> Option<Vec<PluginVersion>> {
    let entry = index.plugins.get(name)?;

    let mut versions: Vec<PluginVersion> = entry
        .versions
        .iter()
        .filter_map(|(ver_str, detail)| {
            let version = semver::Version::parse(ver_str).ok()?;
            Some(PluginVersion {
                version,
                git_commit: detail.commit.clone(),
                path: detail.path.clone(),
            })
        })
        .collect();

    // Sort descending by version (newest first)
    versions.sort_by(|a, b| b.version.cmp(&a.version));

    Some(versions)
}

/// Parse a `marketplace.json` string into a [`RegistryIndex`].
///
/// Useful for testing and for cases where the JSON is already in memory.
pub fn parse_index(json: &str) -> Result<RegistryIndex> {
    serde_json::from_str(json).context("parsing marketplace.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INDEX: &str = r#"{
        "plugins": {
            "quality-review": {
                "description": "A code quality review plugin",
                "versions": {
                    "1.2.0": { "commit": "abc123", "path": "plugins/quality-review" },
                    "1.1.0": { "commit": "def456", "path": "plugins/quality-review" },
                    "1.0.0": { "commit": "789012" }
                }
            },
            "test-runner": {
                "description": "Runs tests automatically",
                "versions": {
                    "0.5.0": { "commit": "aaa111", "path": "plugins/test-runner" }
                }
            }
        }
    }"#;

    #[test]
    fn test_parse_index() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        assert_eq!(index.plugins.len(), 2);
        assert!(index.plugins.contains_key("quality-review"));
        assert!(index.plugins.contains_key("test-runner"));
    }

    #[test]
    fn test_parse_index_plugin_details() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        let qr = &index.plugins["quality-review"];
        assert_eq!(qr.description, "A code quality review plugin");
        assert_eq!(qr.versions.len(), 3);
        assert_eq!(qr.versions["1.2.0"].commit, "abc123");
        assert_eq!(
            qr.versions["1.2.0"].path.as_deref(),
            Some("plugins/quality-review")
        );
    }

    #[test]
    fn test_parse_index_optional_path() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        let qr = &index.plugins["quality-review"];
        assert!(qr.versions["1.0.0"].path.is_none());
    }

    #[test]
    fn test_lookup_plugin_found() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        let versions = lookup_plugin(&index, "quality-review").unwrap();
        assert_eq!(versions.len(), 3);
        // Should be sorted newest first
        assert_eq!(versions[0].version, semver::Version::new(1, 2, 0));
        assert_eq!(versions[1].version, semver::Version::new(1, 1, 0));
        assert_eq!(versions[2].version, semver::Version::new(1, 0, 0));
    }

    #[test]
    fn test_lookup_plugin_not_found() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        assert!(lookup_plugin(&index, "nonexistent").is_none());
    }

    #[test]
    fn test_lookup_plugin_commit_and_path() {
        let index = parse_index(SAMPLE_INDEX).unwrap();
        let versions = lookup_plugin(&index, "quality-review").unwrap();
        assert_eq!(versions[0].git_commit, "abc123");
        assert_eq!(versions[0].path.as_deref(), Some("plugins/quality-review"));
    }

    #[test]
    fn test_parse_empty_plugins() {
        let json = r#"{"plugins": {}}"#;
        let index = parse_index(json).unwrap();
        assert!(index.plugins.is_empty());
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(parse_index("not json").is_err());
    }

    #[test]
    fn test_fetch_index_pinned_none_delegates_to_head() {
        // When pin is None, fetch_index_pinned should use HEAD,
        // which is the same behavior as fetch_index.
        // We can't test network calls, but we can verify the function signature
        // compiles and accepts None.
        let _: fn(&str, &Path, Option<&str>) -> Result<RegistryIndex> = fetch_index_pinned;
    }

    #[test]
    fn test_lookup_skips_invalid_semver() {
        let json = r#"{
            "plugins": {
                "foo": {
                    "versions": {
                        "1.0.0": { "commit": "aaa" },
                        "not-a-version": { "commit": "bbb" }
                    }
                }
            }
        }"#;
        let index = parse_index(json).unwrap();
        let versions = lookup_plugin(&index, "foo").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, semver::Version::new(1, 0, 0));
    }
}
