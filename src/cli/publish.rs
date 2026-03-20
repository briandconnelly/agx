//! `agx publish` — publish an asset to a marketplace registry.

use agx::config;
use agx::manifest;
use agx::manifest::types::AgentEnvironment;
use agx::sources::registry::{PluginEntry, RegistryIndex, VersionDetail};
use anyhow::{bail, Context};
use clap::Args;
use std::collections::BTreeMap;
use std::path::Path;

/// Arguments for the `agx publish` command.
#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Registry to publish to
    #[arg(long)]
    pub registry: String,

    /// Version to publish (must be valid semver)
    #[arg(long)]
    pub version: String,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,
}

/// Run the `agx publish` command.
pub fn run(args: PublishArgs) -> anyhow::Result<()> {
    // Validate semver
    validate_semver(&args.version)?;

    // Load manifest
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let manifest_path = config::find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in current or parent directories",
            config::MANIFEST_FILE
        )
    })?;
    let manifest = manifest::load(&manifest_path)
        .context("failed to load agent-environment.yml — are you in an agx project?")?;

    // Validate manifest has description
    validate_manifest(&manifest)?;

    // Look up registry URL
    let registry_url = lookup_registry(&manifest, &args.registry)?;

    // Get current project commit SHA
    let commit_sha = get_current_commit()?;

    // Verify working tree is clean
    verify_clean_working_tree()?;

    // Confirm unless --yes
    if !args.yes {
        let prompt = format!(
            "Publish {}@{} to registry '{}'?",
            manifest.name, args.version, args.registry
        );
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(&prompt)
            .default(false)
            .interact()
            .context("failed to read confirmation")?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Clone registry to temp dir
    let temp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let registry_dir = temp_dir.path().join("registry");

    let status = std::process::Command::new("git")
        .args(["clone", &registry_url, registry_dir.to_str().unwrap()])
        .status()
        .context("failed to clone registry")?;
    if !status.success() {
        bail!("failed to clone registry at {}", registry_url);
    }

    // Read existing marketplace.json (or create empty)
    let marketplace_path = registry_dir.join("marketplace.json");
    let mut index = if marketplace_path.exists() {
        let content =
            std::fs::read_to_string(&marketplace_path).context("reading marketplace.json")?;
        serde_json::from_str::<RegistryIndex>(&content).context("parsing marketplace.json")?
    } else {
        RegistryIndex {
            plugins: BTreeMap::new(),
        }
    };

    // Check version doesn't already exist
    validate_version_not_exists(&index, &manifest.name, &args.version)?;

    // Update index
    update_index(&mut index, &manifest.name, &manifest.description, &args.version, &commit_sha);

    // Write updated marketplace.json
    let json = serde_json::to_string_pretty(&index).context("serializing marketplace.json")?;
    std::fs::write(&marketplace_path, format!("{json}\n"))
        .context("writing marketplace.json")?;

    // Commit and push
    commit_and_push(&registry_dir, &manifest.name, &args.version)?;

    println!(
        "Published {}@{} to registry '{}'",
        manifest.name, args.version, args.registry
    );

    Ok(())
}

/// Validate that a version string is valid semver.
fn validate_semver(version: &str) -> anyhow::Result<()> {
    semver::Version::parse(version)
        .map_err(|e| anyhow::anyhow!("invalid semver version '{}': {}", version, e))?;
    Ok(())
}

/// Validate the manifest has the required fields for publishing.
fn validate_manifest(manifest: &AgentEnvironment) -> anyhow::Result<()> {
    if manifest.description.as_ref().is_none_or(|d| d.trim().is_empty()) {
        bail!("manifest must have a non-empty 'description' to publish");
    }
    Ok(())
}

/// Look up the registry URL from the manifest's registries section.
fn lookup_registry(manifest: &AgentEnvironment, registry_name: &str) -> anyhow::Result<String> {
    let registry_config = manifest
        .registries
        .get(registry_name)
        .with_context(|| format!("registry '{}' not found in manifest", registry_name))?;
    Ok(registry_config.url.clone())
}

/// Get the current HEAD commit SHA from the project's git repo.
fn get_current_commit() -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .context("failed to run git rev-parse HEAD — is this a git repository?")?;
    if !output.status.success() {
        bail!("failed to get current commit — is this a git repository?");
    }
    Ok(String::from_utf8(output.stdout)
        .context("invalid UTF-8 in git output")?
        .trim()
        .to_string())
}

/// Verify the working tree has no uncommitted changes.
fn verify_clean_working_tree() -> anyhow::Result<()> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("failed to run git status")?;
    if !output.status.success() {
        bail!("failed to check working tree status");
    }
    let stdout = String::from_utf8(output.stdout).context("invalid UTF-8 in git output")?;
    if !stdout.trim().is_empty() {
        bail!("working tree has uncommitted changes — commit or stash them before publishing");
    }
    Ok(())
}

/// Check that the version doesn't already exist in the registry index.
fn validate_version_not_exists(
    index: &RegistryIndex,
    name: &str,
    version: &str,
) -> anyhow::Result<()> {
    if let Some(entry) = index.plugins.get(name) {
        if entry.versions.contains_key(version) {
            bail!(
                "version {} of '{}' already exists in the registry",
                version,
                name
            );
        }
    }
    Ok(())
}

/// Update the registry index with a new plugin version.
fn update_index(
    index: &mut RegistryIndex,
    name: &str,
    description: &Option<String>,
    version: &str,
    commit: &str,
) {
    let entry = index
        .plugins
        .entry(name.to_string())
        .or_insert_with(|| PluginEntry {
            description: description.clone().unwrap_or_default(),
            versions: BTreeMap::new(),
        });

    // Update description if it changed
    if let Some(desc) = description {
        entry.description = desc.clone();
    }

    entry.versions.insert(
        version.to_string(),
        VersionDetail {
            commit: commit.to_string(),
            path: None,
        },
    );
}

/// Commit the marketplace.json change and push to remote.
fn commit_and_push(registry_dir: &Path, name: &str, version: &str) -> anyhow::Result<()> {
    let msg = format!("publish {}@{}", name, version);

    let status = std::process::Command::new("git")
        .args(["add", "marketplace.json"])
        .current_dir(registry_dir)
        .status()
        .context("git add failed")?;
    if !status.success() {
        bail!("git add failed in registry repo");
    }

    let status = std::process::Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(registry_dir)
        .status()
        .context("git commit failed")?;
    if !status.success() {
        bail!("git commit failed in registry repo");
    }

    let status = std::process::Command::new("git")
        .args(["push"])
        .current_dir(registry_dir)
        .status()
        .context("git push failed")?;
    if !status.success() {
        bail!("git push failed — do you have write access to the registry?");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_semver_valid() {
        assert!(validate_semver("1.0.0").is_ok());
        assert!(validate_semver("0.1.0").is_ok());
        assert!(validate_semver("1.2.3-alpha.1").is_ok());
        assert!(validate_semver("1.0.0+build.123").is_ok());
    }

    #[test]
    fn test_validate_semver_invalid() {
        assert!(validate_semver("not-a-version").is_err());
        assert!(validate_semver("1.0").is_err());
        assert!(validate_semver("1").is_err());
        assert!(validate_semver("").is_err());
        assert!(validate_semver("v1.0.0").is_err());
    }

    #[test]
    fn test_validate_manifest_missing_description() {
        let manifest = AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };
        let result = validate_manifest(&manifest);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("description")
        );
    }

    #[test]
    fn test_validate_manifest_empty_description() {
        let manifest = AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: Some("   ".to_string()),
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn test_validate_manifest_with_description() {
        let manifest = AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: Some("A test plugin".to_string()),
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn test_lookup_registry_found() {
        use agx::manifest::types::RegistryConfig;
        let mut manifest = AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };
        manifest.registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://github.com/example/registry".to_string(),
                auth: None,
                pin: None,
            },
        );
        let url = lookup_registry(&manifest, "official").unwrap();
        assert_eq!(url, "https://github.com/example/registry");
    }

    #[test]
    fn test_lookup_registry_not_found() {
        let manifest = AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };
        assert!(lookup_registry(&manifest, "nonexistent").is_err());
    }

    #[test]
    fn test_validate_version_not_exists_ok() {
        let index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        assert!(validate_version_not_exists(&index, "my-plugin", "1.0.0").is_ok());
    }

    #[test]
    fn test_validate_version_not_exists_different_version() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let mut versions = BTreeMap::new();
        versions.insert(
            "1.0.0".to_string(),
            VersionDetail {
                commit: "abc123".to_string(),
                path: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "test".to_string(),
                versions,
            },
        );
        assert!(validate_version_not_exists(&index, "my-plugin", "2.0.0").is_ok());
    }

    #[test]
    fn test_validate_version_already_exists() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let mut versions = BTreeMap::new();
        versions.insert(
            "1.0.0".to_string(),
            VersionDetail {
                commit: "abc123".to_string(),
                path: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "test".to_string(),
                versions,
            },
        );
        let result = validate_version_not_exists(&index, "my-plugin", "1.0.0");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_update_index_new_plugin() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let desc = Some("A cool plugin".to_string());
        update_index(&mut index, "my-plugin", &desc, "1.0.0", "abc123");

        assert!(index.plugins.contains_key("my-plugin"));
        let entry = &index.plugins["my-plugin"];
        assert_eq!(entry.description, "A cool plugin");
        assert_eq!(entry.versions.len(), 1);
        assert_eq!(entry.versions["1.0.0"].commit, "abc123");
        assert!(entry.versions["1.0.0"].path.is_none());
    }

    #[test]
    fn test_update_index_existing_plugin_new_version() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let mut versions = BTreeMap::new();
        versions.insert(
            "1.0.0".to_string(),
            VersionDetail {
                commit: "abc123".to_string(),
                path: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "old description".to_string(),
                versions,
            },
        );

        let desc = Some("new description".to_string());
        update_index(&mut index, "my-plugin", &desc, "2.0.0", "def456");

        let entry = &index.plugins["my-plugin"];
        assert_eq!(entry.description, "new description");
        assert_eq!(entry.versions.len(), 2);
        assert_eq!(entry.versions["1.0.0"].commit, "abc123");
        assert_eq!(entry.versions["2.0.0"].commit, "def456");
    }

    #[test]
    fn test_update_index_roundtrip_json() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let desc = Some("A plugin".to_string());
        update_index(&mut index, "test-plugin", &desc, "1.0.0", "sha1");
        update_index(&mut index, "test-plugin", &desc, "1.1.0", "sha2");

        let json = serde_json::to_string_pretty(&index).unwrap();
        let parsed: RegistryIndex = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.plugins.len(), 1);
        assert_eq!(parsed.plugins["test-plugin"].versions.len(), 2);
        assert_eq!(parsed.plugins["test-plugin"].versions["1.0.0"].commit, "sha1");
        assert_eq!(parsed.plugins["test-plugin"].versions["1.1.0"].commit, "sha2");
    }
}
