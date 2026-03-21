//! Marketplace registry client — fetch and cache registry indexes.
//!
//! A registry is a git repository containing a `.claude-plugin/marketplace.json`
//! index file that lists available plugins. Each plugin directory contains its
//! own `.claude-plugin/plugin.json` with version metadata.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::git;

// ---------------------------------------------------------------------------
// Public types — the normalized view consumed by the resolver
// ---------------------------------------------------------------------------

/// Parsed registry index — the normalized view used by the resolver.
///
/// Built from the raw marketplace format, which uses an array of plugin
/// entries, each pointing to a source directory within the registry repo.
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
    /// Plugin author name.
    #[serde(default)]
    pub author: Option<String>,
    /// Plugin homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Source code repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Keywords for discovery.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Tags for searchability.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Category for organization.
    #[serde(default)]
    pub category: Option<String>,
    /// Whether plugin.json is the authority for component definitions.
    #[serde(default)]
    pub strict: Option<bool>,
}

/// Details for a specific version of a plugin in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDetail {
    /// The git commit SHA for this version.
    pub commit: String,
    /// Path within the registry repo where this plugin's assets live.
    #[serde(default)]
    pub path: Option<String>,
    /// How to fetch this plugin.
    #[serde(default)]
    pub source_kind: Option<PluginSourceKind>,
}

/// Describes where a plugin's assets live and how to fetch them.
///
/// This is the normalized representation used internally. It is derived
/// from the raw `source` field in marketplace.json plugin entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginSourceKind {
    /// Relative path within the same registry/marketplace repo.
    RegistryLocal {
        /// Path within the registry repo (e.g. "analytics").
        path: String,
    },
    /// GitHub shorthand source (`owner/repo`).
    GitHub {
        /// Repository in `owner/repo` format.
        repo: String,
        /// Branch or tag ref.
        git_ref: Option<String>,
        /// Exact commit SHA.
        sha: Option<String>,
    },
    /// Arbitrary git URL source.
    GitUrl {
        /// Full git URL.
        url: String,
        /// Branch or tag ref.
        git_ref: Option<String>,
        /// Exact commit SHA.
        sha: Option<String>,
    },
    /// Subdirectory within a git repository.
    GitSubdir {
        /// Git URL or GitHub `owner/repo` shorthand.
        url: String,
        /// Subdirectory path within the repo.
        path: String,
        /// Branch or tag ref.
        git_ref: Option<String>,
        /// Exact commit SHA.
        sha: Option<String>,
    },
    /// npm package source.
    Npm {
        /// Package name (e.g. `@acme/claude-plugin`).
        package: String,
        /// Version or version range.
        version: Option<String>,
        /// Custom npm registry URL.
        registry: Option<String>,
    },
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
    /// Source kind for this version.
    pub source_kind: Option<PluginSourceKind>,
}

// ---------------------------------------------------------------------------
// Raw marketplace.json format (as found in real registries)
// ---------------------------------------------------------------------------

/// Raw marketplace.json — the on-disk format in `.claude-plugin/marketplace.json`.
#[derive(Debug, Clone, Deserialize)]
struct RawMarketplace {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    owner: Option<RawMarketplaceOwner>,
    #[serde(default)]
    metadata: Option<RawMarketplaceMetadata>,
    /// Plugins listed as an array (real format) or map (legacy format).
    plugins: RawPlugins,
}

/// Marketplace owner information.
#[derive(Debug, Clone, Deserialize)]
struct RawMarketplaceOwner {
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    email: Option<String>,
}

/// Optional marketplace-level metadata.
#[derive(Debug, Clone, Deserialize)]
struct RawMarketplaceMetadata {
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,
    /// Base directory prepended to relative plugin source paths.
    #[serde(rename = "pluginRoot", default)]
    plugin_root: Option<String>,
}

/// Plugins can be either an array (real-world format) or a map (legacy agx format).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawPlugins {
    /// Real-world format: array of plugin entries.
    Array(Vec<RawPluginEntry>),
    /// Legacy format: map of plugin name → plugin metadata with versions.
    Map(BTreeMap<String, PluginEntry>),
}

/// The `source` field on a plugin entry: either a relative path string or
/// a structured object with a `source` discriminator.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawPluginSource {
    /// Relative path string: `"./plugins/my-plugin"`
    RelativePath(String),
    /// Object source with a `source` discriminator field.
    Object(RawPluginSourceObject),
}

/// Structured plugin source — discriminated via `#[serde(tag = "source")]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "source")]
enum RawPluginSourceObject {
    /// GitHub shorthand: `{source: "github", repo: "owner/repo"}`.
    #[serde(rename = "github")]
    GitHub {
        repo: String,
        #[serde(rename = "ref", default)]
        git_ref: Option<String>,
        #[serde(default)]
        sha: Option<String>,
    },
    /// Arbitrary git URL: `{source: "url", url: "https://..."}`.
    #[serde(rename = "url")]
    GitUrl {
        url: String,
        #[serde(rename = "ref", default)]
        git_ref: Option<String>,
        #[serde(default)]
        sha: Option<String>,
    },
    /// Git subdirectory: `{source: "git-subdir", url: "...", path: "..."}`.
    #[serde(rename = "git-subdir")]
    GitSubdir {
        url: String,
        path: String,
        #[serde(rename = "ref", default)]
        git_ref: Option<String>,
        #[serde(default)]
        sha: Option<String>,
    },
    /// npm package: `{source: "npm", package: "@acme/plugin"}`.
    #[serde(rename = "npm")]
    Npm {
        package: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        registry: Option<String>,
    },
}

/// A single plugin entry in the array-based marketplace format.
#[derive(Debug, Clone, Deserialize)]
struct RawPluginEntry {
    /// Plugin name.
    name: String,
    /// Description.
    #[serde(default)]
    description: Option<String>,
    /// Where to fetch the plugin (string path or structured object).
    #[serde(default)]
    source: Option<RawPluginSource>,
    /// Version declared inline in the marketplace entry.
    #[serde(default)]
    version: Option<String>,
    /// Category tag.
    #[serde(default)]
    category: Option<String>,
    /// Plugin author info.
    #[serde(default)]
    author: Option<RawAuthor>,
    /// Plugin homepage URL.
    #[serde(default)]
    homepage: Option<String>,
    /// Source repository URL.
    #[serde(default)]
    repository: Option<String>,
    /// SPDX license identifier.
    #[serde(default)]
    license: Option<String>,
    /// Keywords for discovery.
    #[serde(default)]
    keywords: Vec<String>,
    /// Tags for searchability.
    #[serde(default)]
    tags: Vec<String>,
    /// Whether plugin.json is the authority for component definitions.
    #[serde(default)]
    strict: Option<bool>,
    // Passthrough fields — stored for future use but not processed by agx.
    #[serde(default)]
    #[allow(dead_code)]
    commands: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    agents: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    hooks: Option<serde_json::Value>,
    #[serde(rename = "mcpServers", default)]
    #[allow(dead_code)]
    mcp_servers: Option<serde_json::Value>,
    #[serde(rename = "lspServers", default)]
    #[allow(dead_code)]
    lsp_servers: Option<serde_json::Value>,
}

/// Author can be a string or an object with name/email.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawAuthor {
    Name(String),
    Object { name: String, #[serde(default)] email: Option<String> },
}

impl RawAuthor {
    fn name(&self) -> &str {
        match self {
            RawAuthor::Name(s) => s,
            RawAuthor::Object { name, .. } => name,
        }
    }
}

/// Per-plugin metadata from `<plugin-dir>/.claude-plugin/plugin.json`.
#[derive(Debug, Clone, Deserialize)]
struct RawPluginJson {
    #[serde(default)]
    version: Option<String>,
}

// ---------------------------------------------------------------------------
// Fetch and parse
// ---------------------------------------------------------------------------

/// Fetch (or update) a registry index from a git URL and parse it.
///
/// Clones the registry repo into `cache_dir` if it doesn't exist,
/// otherwise fetches updates. Then parses the marketplace index from HEAD.
pub fn fetch_index(url: &str, cache_dir: &Path) -> Result<RegistryIndex> {
    fetch_index_pinned(url, cache_dir, None)
}

/// Fetch a registry index with optional commit pinning.
///
/// If `pin` is `Some`, the index is read from that specific commit SHA
/// instead of HEAD. This ensures reproducibility even if the registry updates.
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

    let json_bytes = git_show(cache_dir, rev, ".claude-plugin/marketplace.json")
        .context("reading .claude-plugin/marketplace.json from registry")?;

    let raw: RawMarketplace = serde_json::from_slice(&json_bytes)
        .context("parsing .claude-plugin/marketplace.json")?;

    build_index(raw, cache_dir, rev)
}

/// Build a `RegistryIndex` from the raw marketplace data.
///
/// For the array format, reads each plugin's `.claude-plugin/plugin.json`
/// from the bare repo to get its version. Applies `metadata.pluginRoot`
/// to relative source paths.
fn build_index(raw: RawMarketplace, cache_dir: &Path, rev: &str) -> Result<RegistryIndex> {
    let plugin_root = raw
        .metadata
        .as_ref()
        .and_then(|m| m.plugin_root.as_deref());

    // Validate pluginRoot if present
    if let Some(root) = plugin_root {
        if root.contains("..") {
            anyhow::bail!("metadata.pluginRoot must not contain '..': {root}");
        }
        if root.starts_with('/') {
            anyhow::bail!("metadata.pluginRoot must be a relative path: {root}");
        }
    }

    match raw.plugins {
        RawPlugins::Map(plugins) => {
            // Legacy format — already in the right shape.
            Ok(RegistryIndex { plugins })
        }
        RawPlugins::Array(entries) => {
            let mut plugins = BTreeMap::new();

            for entry in entries {
                let (source_kind, source_dir) =
                    resolve_plugin_source(&entry, plugin_root);

                // For registry-local sources, try to read plugin.json for version
                let plugin_json_version = match &source_kind {
                    PluginSourceKind::RegistryLocal { path } => {
                        let plugin_json_path =
                            format!("{path}/.claude-plugin/plugin.json");
                        match git_show(cache_dir, rev, &plugin_json_path) {
                            Ok(bytes) => serde_json::from_slice::<RawPluginJson>(&bytes)
                                .ok()
                                .and_then(|p| p.version),
                            Err(_) => None,
                        }
                    }
                    _ => None,
                };

                // Version precedence: plugin.json > marketplace entry > "0.0.0"
                let ver_str = plugin_json_version
                    .or_else(|| entry.version.clone())
                    .unwrap_or_else(|| "0.0.0".to_string());

                // Resolve HEAD commit for registry-local entries
                let commit = match &source_kind {
                    PluginSourceKind::RegistryLocal { .. } => {
                        resolve_head_sha(cache_dir, rev)
                    }
                    PluginSourceKind::GitHub { sha, .. }
                    | PluginSourceKind::GitUrl { sha, .. }
                    | PluginSourceKind::GitSubdir { sha, .. } => {
                        // If SHA is pinned, use it; otherwise empty (resolved later)
                        sha.clone().unwrap_or_default()
                    }
                    PluginSourceKind::Npm { .. } => String::new(),
                };

                let mut versions = BTreeMap::new();
                versions.insert(
                    ver_str,
                    VersionDetail {
                        commit,
                        path: source_dir,
                        source_kind: Some(source_kind),
                    },
                );

                if plugins.contains_key(&entry.name) {
                    anyhow::bail!(
                        "duplicate plugin name '{}' in marketplace",
                        entry.name
                    );
                }

                plugins.insert(
                    entry.name,
                    PluginEntry {
                        description: entry.description.unwrap_or_default(),
                        versions,
                        author: entry.author.as_ref().map(|a| a.name().to_string()),
                        homepage: entry.homepage,
                        repository: entry.repository,
                        license: entry.license,
                        keywords: entry.keywords,
                        tags: entry.tags,
                        category: entry.category,
                        strict: entry.strict,
                    },
                );
            }

            Ok(RegistryIndex { plugins })
        }
    }
}

/// Resolve a raw plugin source into a `PluginSourceKind` and optional path.
///
/// If `plugin_root` is set, it is prepended to relative paths.
fn resolve_plugin_source(
    entry: &RawPluginEntry,
    plugin_root: Option<&str>,
) -> (PluginSourceKind, Option<String>) {
    match &entry.source {
        None => {
            // No source specified — default to "./<name>"
            let path = apply_plugin_root(&entry.name, plugin_root);
            (
                PluginSourceKind::RegistryLocal { path: path.clone() },
                Some(path),
            )
        }
        Some(RawPluginSource::RelativePath(rel)) => {
            let stripped = rel.trim_start_matches("./");
            let path = apply_plugin_root(stripped, plugin_root);
            (
                PluginSourceKind::RegistryLocal { path: path.clone() },
                Some(path),
            )
        }
        Some(RawPluginSource::Object(obj)) => match obj {
            RawPluginSourceObject::GitHub { repo, git_ref, sha } => (
                PluginSourceKind::GitHub {
                    repo: repo.clone(),
                    git_ref: git_ref.clone(),
                    sha: sha.clone(),
                },
                None,
            ),
            RawPluginSourceObject::GitUrl { url, git_ref, sha } => (
                PluginSourceKind::GitUrl {
                    url: url.clone(),
                    git_ref: git_ref.clone(),
                    sha: sha.clone(),
                },
                None,
            ),
            RawPluginSourceObject::GitSubdir {
                url,
                path,
                git_ref,
                sha,
            } => (
                PluginSourceKind::GitSubdir {
                    url: url.clone(),
                    path: path.clone(),
                    git_ref: git_ref.clone(),
                    sha: sha.clone(),
                },
                None,
            ),
            RawPluginSourceObject::Npm {
                package,
                version,
                registry,
            } => (
                PluginSourceKind::Npm {
                    package: package.clone(),
                    version: version.clone(),
                    registry: registry.clone(),
                },
                None,
            ),
        },
    }
}

/// Prepend `plugin_root` to a path if set.
fn apply_plugin_root(path: &str, plugin_root: Option<&str>) -> String {
    match plugin_root {
        Some(root) => {
            let root = root.trim_start_matches("./").trim_end_matches('/');
            format!("{root}/{path}")
        }
        None => path.to_string(),
    }
}

/// Run `git show <rev>:<path>` in a bare repo and return the raw bytes.
fn git_show(repo_path: &Path, rev: &str, path: &str) -> Result<Vec<u8>> {
    let output = std::process::Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .current_dir(repo_path)
        .output()
        .with_context(|| format!("running git show {rev}:{path}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "{} not found at rev '{}' in {}",
            path,
            rev,
            repo_path.display()
        );
    }

    Ok(output.stdout)
}

/// Resolve a rev to a full SHA, falling back to the rev string itself.
fn resolve_head_sha(repo_path: &Path, rev: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => rev.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

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
                source_kind: detail.source_kind.clone(),
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
/// Accepts both the array format (real-world) and map format (legacy).
pub fn parse_index(json: &str) -> Result<RegistryIndex> {
    let raw: RawMarketplace = serde_json::from_str(json)
        .context("parsing marketplace.json")?;

    let plugin_root = raw
        .metadata
        .as_ref()
        .and_then(|m| m.plugin_root.as_deref())
        .map(|s| s.to_string());

    match raw.plugins {
        RawPlugins::Map(plugins) => Ok(RegistryIndex { plugins }),
        RawPlugins::Array(entries) => {
            let mut plugins = BTreeMap::new();
            for entry in entries {
                let (source_kind, source_dir) =
                    resolve_plugin_source(&entry, plugin_root.as_deref());

                // Version: marketplace entry or "0.0.0" (no git repo for parse_index)
                let ver_str = entry.version.clone().unwrap_or_else(|| "0.0.0".to_string());

                let mut versions = BTreeMap::new();
                versions.insert(
                    ver_str,
                    VersionDetail {
                        commit: String::new(),
                        path: source_dir,
                        source_kind: Some(source_kind),
                    },
                );

                if plugins.contains_key(&entry.name) {
                    anyhow::bail!(
                        "duplicate plugin name '{}' in marketplace",
                        entry.name
                    );
                }

                plugins.insert(
                    entry.name,
                    PluginEntry {
                        description: entry.description.unwrap_or_default(),
                        versions,
                        author: entry.author.as_ref().map(|a| a.name().to_string()),
                        homepage: entry.homepage,
                        repository: entry.repository,
                        license: entry.license,
                        keywords: entry.keywords,
                        tags: entry.tags,
                        category: entry.category,
                        strict: entry.strict,
                    },
                );
            }
            Ok(RegistryIndex { plugins })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Legacy map format
    const SAMPLE_INDEX_LEGACY: &str = r#"{
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

    // Real-world array format
    const SAMPLE_INDEX_ARRAY: &str = r#"{
        "name": "my-marketplace",
        "description": "Tools for working with data",
        "version": "1.0.0",
        "plugins": [
            {
                "name": "analytics",
                "description": "Tools for analyzing data",
                "source": "./analytics",
                "category": "Data Analysis"
            },
            {
                "name": "snowflake",
                "description": "MCP server for Snowflake",
                "source": "./snowflake",
                "category": "Data Analysis"
            }
        ]
    }"#;

    #[test]
    fn test_parse_legacy_index() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
        assert_eq!(index.plugins.len(), 2);
        assert!(index.plugins.contains_key("quality-review"));
        assert!(index.plugins.contains_key("test-runner"));
    }

    #[test]
    fn test_parse_legacy_index_plugin_details() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
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
    fn test_parse_legacy_index_optional_path() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
        let qr = &index.plugins["quality-review"];
        assert!(qr.versions["1.0.0"].path.is_none());
    }

    #[test]
    fn test_parse_array_index() {
        let index = parse_index(SAMPLE_INDEX_ARRAY).unwrap();
        assert_eq!(index.plugins.len(), 2);
        assert!(index.plugins.contains_key("analytics"));
        assert!(index.plugins.contains_key("snowflake"));
    }

    #[test]
    fn test_parse_array_index_details() {
        let index = parse_index(SAMPLE_INDEX_ARRAY).unwrap();
        let analytics = &index.plugins["analytics"];
        assert_eq!(analytics.description, "Tools for analyzing data");
        assert!(analytics.versions.contains_key("0.0.0"));
        assert_eq!(
            analytics.versions["0.0.0"].path.as_deref(),
            Some("analytics")
        );
    }

    #[test]
    fn test_parse_array_source_stripped() {
        let index = parse_index(SAMPLE_INDEX_ARRAY).unwrap();
        let snowflake = &index.plugins["snowflake"];
        assert_eq!(
            snowflake.versions["0.0.0"].path.as_deref(),
            Some("snowflake")
        );
    }

    #[test]
    fn test_lookup_plugin_found() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
        let versions = lookup_plugin(&index, "quality-review").unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].version, semver::Version::new(1, 2, 0));
        assert_eq!(versions[1].version, semver::Version::new(1, 1, 0));
        assert_eq!(versions[2].version, semver::Version::new(1, 0, 0));
    }

    #[test]
    fn test_lookup_plugin_not_found() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
        assert!(lookup_plugin(&index, "nonexistent").is_none());
    }

    #[test]
    fn test_lookup_plugin_commit_and_path() {
        let index = parse_index(SAMPLE_INDEX_LEGACY).unwrap();
        let versions = lookup_plugin(&index, "quality-review").unwrap();
        assert_eq!(versions[0].git_commit, "abc123");
        assert_eq!(versions[0].path.as_deref(), Some("plugins/quality-review"));
    }

    #[test]
    fn test_parse_empty_plugins_map() {
        let json = r#"{"plugins": {}}"#;
        let index = parse_index(json).unwrap();
        assert!(index.plugins.is_empty());
    }

    #[test]
    fn test_parse_empty_plugins_array() {
        let json = r#"{"plugins": []}"#;
        let index = parse_index(json).unwrap();
        assert!(index.plugins.is_empty());
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(parse_index("not json").is_err());
    }

    #[test]
    fn test_fetch_index_pinned_none_delegates_to_head() {
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

    // --- Source type parsing tests ---

    #[test]
    fn test_parse_github_source() {
        let json = r#"{
            "name": "test-marketplace",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "gh-plugin",
                "source": {
                    "source": "github",
                    "repo": "acme/plugin",
                    "ref": "v2.0.0",
                    "sha": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"
                },
                "description": "A GitHub-sourced plugin",
                "version": "2.0.0"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        let plugin = &index.plugins["gh-plugin"];
        assert_eq!(plugin.description, "A GitHub-sourced plugin");
        assert!(plugin.versions.contains_key("2.0.0"));

        let detail = &plugin.versions["2.0.0"];
        assert!(detail.path.is_none()); // GitHub sources have no local path

        match detail.source_kind.as_ref().unwrap() {
            PluginSourceKind::GitHub { repo, git_ref, sha } => {
                assert_eq!(repo, "acme/plugin");
                assert_eq!(git_ref.as_deref(), Some("v2.0.0"));
                assert_eq!(
                    sha.as_deref(),
                    Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0")
                );
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_url_source() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "git-plugin",
                "source": {
                    "source": "url",
                    "url": "https://gitlab.com/team/plugin.git"
                }
            }]
        }"#;
        let index = parse_index(json).unwrap();
        let detail = &index.plugins["git-plugin"].versions["0.0.0"];
        match detail.source_kind.as_ref().unwrap() {
            PluginSourceKind::GitUrl { url, git_ref, sha } => {
                assert_eq!(url, "https://gitlab.com/team/plugin.git");
                assert!(git_ref.is_none());
                assert!(sha.is_none());
            }
            other => panic!("expected GitUrl source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_subdir_source() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "subdir-plugin",
                "source": {
                    "source": "git-subdir",
                    "url": "https://github.com/acme/monorepo.git",
                    "path": "tools/claude-plugin",
                    "ref": "main"
                }
            }]
        }"#;
        let index = parse_index(json).unwrap();
        let detail = &index.plugins["subdir-plugin"].versions["0.0.0"];
        match detail.source_kind.as_ref().unwrap() {
            PluginSourceKind::GitSubdir {
                url,
                path,
                git_ref,
                sha,
            } => {
                assert_eq!(url, "https://github.com/acme/monorepo.git");
                assert_eq!(path, "tools/claude-plugin");
                assert_eq!(git_ref.as_deref(), Some("main"));
                assert!(sha.is_none());
            }
            other => panic!("expected GitSubdir source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_npm_source() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "npm-plugin",
                "source": {
                    "source": "npm",
                    "package": "@acme/claude-plugin",
                    "version": "^2.0.0"
                }
            }]
        }"#;
        let index = parse_index(json).unwrap();
        let detail = &index.plugins["npm-plugin"].versions["0.0.0"];
        match detail.source_kind.as_ref().unwrap() {
            PluginSourceKind::Npm {
                package,
                version,
                registry,
            } => {
                assert_eq!(package, "@acme/claude-plugin");
                assert_eq!(version.as_deref(), Some("^2.0.0"));
                assert!(registry.is_none());
            }
            other => panic!("expected Npm source, got {:?}", other),
        }
    }

    // --- Plugin metadata tests ---

    #[test]
    fn test_parse_full_metadata() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Team", "email": "team@example.com" },
            "plugins": [{
                "name": "rich-plugin",
                "source": "./rich-plugin",
                "description": "A richly-described plugin",
                "version": "1.5.0",
                "author": { "name": "Jane Doe", "email": "jane@example.com" },
                "homepage": "https://example.com/plugin",
                "repository": "https://github.com/example/plugin",
                "license": "MIT",
                "keywords": ["testing", "quality"],
                "tags": ["stable", "recommended"],
                "category": "code-quality",
                "strict": false
            }]
        }"#;
        let index = parse_index(json).unwrap();
        let plugin = &index.plugins["rich-plugin"];
        assert_eq!(plugin.description, "A richly-described plugin");
        assert_eq!(plugin.author.as_deref(), Some("Jane Doe"));
        assert_eq!(plugin.homepage.as_deref(), Some("https://example.com/plugin"));
        assert_eq!(
            plugin.repository.as_deref(),
            Some("https://github.com/example/plugin")
        );
        assert_eq!(plugin.license.as_deref(), Some("MIT"));
        assert_eq!(plugin.keywords, vec!["testing", "quality"]);
        assert_eq!(plugin.tags, vec!["stable", "recommended"]);
        assert_eq!(plugin.category.as_deref(), Some("code-quality"));
        assert_eq!(plugin.strict, Some(false));
        // Inline version used
        assert!(plugin.versions.contains_key("1.5.0"));
    }

    #[test]
    fn test_parse_author_as_string() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "p",
                "source": "./p",
                "author": "Bob Smith"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert_eq!(index.plugins["p"].author.as_deref(), Some("Bob Smith"));
    }

    // --- pluginRoot tests ---

    #[test]
    fn test_plugin_root_prepended() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "metadata": { "pluginRoot": "./plugins" },
            "plugins": [{
                "name": "formatter",
                "source": "formatter"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert_eq!(
            index.plugins["formatter"].versions["0.0.0"].path.as_deref(),
            Some("plugins/formatter")
        );
    }

    #[test]
    fn test_plugin_root_with_relative_source() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "metadata": { "pluginRoot": "./tools" },
            "plugins": [{
                "name": "linter",
                "source": "./linter"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert_eq!(
            index.plugins["linter"].versions["0.0.0"].path.as_deref(),
            Some("tools/linter")
        );
    }

    #[test]
    fn test_plugin_root_not_applied_to_object_sources() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "metadata": { "pluginRoot": "./plugins" },
            "plugins": [{
                "name": "external",
                "source": {
                    "source": "github",
                    "repo": "acme/external"
                }
            }]
        }"#;
        let index = parse_index(json).unwrap();
        // Object sources should not have pluginRoot applied
        assert!(index.plugins["external"].versions["0.0.0"].path.is_none());
    }

    // --- Version precedence tests ---

    #[test]
    fn test_marketplace_version_used_without_git() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "versioned",
                "source": "./versioned",
                "version": "3.2.1"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert!(index.plugins["versioned"].versions.contains_key("3.2.1"));
    }

    #[test]
    fn test_no_source_defaults_to_name() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "my-plugin"
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert_eq!(
            index.plugins["my-plugin"].versions["0.0.0"].path.as_deref(),
            Some("my-plugin")
        );
    }

    // --- Mixed source types ---

    #[test]
    fn test_parse_mixed_sources() {
        let json = r#"{
            "name": "company-tools",
            "owner": { "name": "DevTools Team" },
            "plugins": [
                {
                    "name": "local-plugin",
                    "source": "./plugins/local",
                    "version": "1.0.0"
                },
                {
                    "name": "github-plugin",
                    "source": { "source": "github", "repo": "company/plugin" },
                    "version": "2.0.0"
                },
                {
                    "name": "npm-plugin",
                    "source": { "source": "npm", "package": "@co/plugin" }
                }
            ]
        }"#;
        let index = parse_index(json).unwrap();
        assert_eq!(index.plugins.len(), 3);

        // Local plugin
        let local = &index.plugins["local-plugin"];
        assert!(local.versions.contains_key("1.0.0"));
        assert_eq!(local.versions["1.0.0"].path.as_deref(), Some("plugins/local"));

        // GitHub plugin
        let gh = &index.plugins["github-plugin"];
        assert!(gh.versions.contains_key("2.0.0"));
        assert!(gh.versions["2.0.0"].path.is_none());

        // npm plugin
        let npm = &index.plugins["npm-plugin"];
        assert!(npm.versions.contains_key("0.0.0"));
    }

    // --- Passthrough fields ---

    // --- Duplicate detection ---

    #[test]
    fn test_duplicate_plugin_name_rejected() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [
                { "name": "dup", "source": "./a" },
                { "name": "dup", "source": "./b" }
            ]
        }"#;
        let result = parse_index(json);
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("duplicate plugin name 'dup'"), "got: {err}");
    }

    // --- Passthrough fields ---

    #[test]
    fn test_passthrough_fields_dont_break_parsing() {
        let json = r#"{
            "name": "test",
            "owner": { "name": "Test" },
            "plugins": [{
                "name": "enterprise-tools",
                "source": "./enterprise",
                "commands": ["./commands/core/", "./commands/extra/"],
                "agents": ["./agents/reviewer.md"],
                "hooks": {
                    "PostToolUse": [{ "matcher": "Write", "hooks": [{ "type": "command", "command": "validate.sh" }] }]
                },
                "mcpServers": {
                    "db": { "command": "db-server", "args": ["--config", "config.json"] }
                },
                "lspServers": {
                    "ts": { "command": "typescript-language-server" }
                }
            }]
        }"#;
        let index = parse_index(json).unwrap();
        assert!(index.plugins.contains_key("enterprise-tools"));
    }
}
