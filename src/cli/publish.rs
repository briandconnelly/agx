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
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in current or parent directories",
            config::MANIFEST_FILE
        )
    })?;
    let manifest = manifest::load(&manifest_path)
        .context("loading agent-environment.yml — are you in an agx project?")?;

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
            .context("reading confirmation")?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Clone registry to temp dir
    let temp_dir = tempfile::tempdir().context("creating temp directory")?;
    let registry_dir = temp_dir.path().join("registry");

    let status = std::process::Command::new("git")
        .arg("clone")
        .arg(&registry_url)
        .arg(&registry_dir)
        .status()
        .context("cloning registry")?;
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
        .context("running git rev-parse HEAD — is this a git repository?")?;
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
        .context("running git status")?;
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
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: Vec::new(),
            tags: Vec::new(),
            category: None,
            strict: None,
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
            source_kind: None,
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

    /// Create a minimal test manifest via YAML parsing.
    fn test_manifest() -> AgentEnvironment {
        serde_yaml::from_str("name: test\nbackends:\n  claude-code: {}\n").unwrap()
    }

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
        let manifest = test_manifest();
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
        let mut manifest = test_manifest();
        manifest.description = Some("   ".to_string());
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn test_validate_manifest_with_description() {
        let mut manifest = test_manifest();
        manifest.description = Some("A test plugin".to_string());
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn test_lookup_registry_found() {
        use agx::manifest::types::RegistryConfig;
        let mut manifest = test_manifest();
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
        let manifest = test_manifest();
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
                source_kind: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "test".to_string(),
                versions,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                tags: Vec::new(),
                category: None,
                strict: None,
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
                source_kind: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "test".to_string(),
                versions,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                tags: Vec::new(),
                category: None,
                strict: None,
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
                source_kind: None,
            },
        );
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "old description".to_string(),
                versions,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                tags: Vec::new(),
                category: None,
                strict: None,
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

    // -----------------------------------------------------------------------
    // update_index additional tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_index_none_description() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        let desc: Option<String> = None;
        update_index(&mut index, "no-desc-plugin", &desc, "1.0.0", "abc123");

        let entry = &index.plugins["no-desc-plugin"];
        // With None description, or_insert_with uses unwrap_or_default => empty string
        assert_eq!(entry.description, "");
        assert_eq!(entry.versions.len(), 1);
    }

    #[test]
    fn test_update_index_description_update() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };

        // First insert with one description
        let desc1 = Some("Original description".to_string());
        update_index(&mut index, "my-plugin", &desc1, "1.0.0", "aaa");
        assert_eq!(index.plugins["my-plugin"].description, "Original description");

        // Update with a new description
        let desc2 = Some("Updated description".to_string());
        update_index(&mut index, "my-plugin", &desc2, "2.0.0", "bbb");
        assert_eq!(index.plugins["my-plugin"].description, "Updated description");
        assert_eq!(index.plugins["my-plugin"].versions.len(), 2);
    }

    #[test]
    fn test_update_index_none_description_does_not_overwrite() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };

        // First insert with a description
        let desc1 = Some("Keep this".to_string());
        update_index(&mut index, "my-plugin", &desc1, "1.0.0", "aaa");

        // Update with None description — should not overwrite
        let desc_none: Option<String> = None;
        update_index(&mut index, "my-plugin", &desc_none, "2.0.0", "bbb");
        assert_eq!(index.plugins["my-plugin"].description, "Keep this");
    }

    // -----------------------------------------------------------------------
    // validate_version_not_exists with empty versions map
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_version_not_exists_empty_versions() {
        let mut index = RegistryIndex {
            plugins: BTreeMap::new(),
        };
        index.plugins.insert(
            "my-plugin".to_string(),
            PluginEntry {
                description: "test".to_string(),
                versions: BTreeMap::new(),
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                tags: Vec::new(),
                category: None,
                strict: None,
            },
        );
        // Plugin exists but has no versions — should be ok for any version.
        assert!(validate_version_not_exists(&index, "my-plugin", "1.0.0").is_ok());
    }

    // -----------------------------------------------------------------------
    // get_current_commit — create git repo in tempdir
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_current_commit_in_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();

        // Configure git user for this repo
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .unwrap();

        // Create a file, add, and commit
        std::fs::write(path.join("test.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();

        // Run get_current_commit from within the temp dir
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        let result = get_current_commit();
        std::env::set_current_dir(original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
        let sha = result.unwrap();
        // SHA should be 40 hex characters
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -----------------------------------------------------------------------
    // verify_clean_working_tree tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_verify_clean_working_tree_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // Initialize git repo with a commit
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .unwrap();
        std::fs::write(path.join("test.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        let result = verify_clean_working_tree();
        std::env::set_current_dir(original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // run() integration tests using local bare git repos
    // -----------------------------------------------------------------------

    /// Helper: create a bare git repo to serve as a "registry" remote.
    fn create_bare_registry(dir: &std::path::Path) {
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    /// Helper: create a project dir with manifest and git repo.
    fn setup_publish_project(dir: &std::path::Path, registry_url: &str) {
        // init git
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();

        // write manifest with description and registry
        let manifest = format!(
            r#"name: my-publish-test
description: A test plugin for publishing
backends:
  claude-code: {{}}
registries:
  local-reg:
    url: {}
"#,
            registry_url
        );
        std::fs::write(dir.join("agent-environment.yml"), manifest).unwrap();

        // add and commit
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn test_run_full_publish_flow() {
        // Create a bare registry repo
        let registry_tmp = tempfile::tempdir().unwrap();
        let bare_dir = registry_tmp.path().join("registry.git");
        std::fs::create_dir_all(&bare_dir).unwrap();
        create_bare_registry(&bare_dir);

        // Clone it and add an initial commit so pushes work
        let init_clone = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", &bare_dir.display().to_string(), "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::fs::write(init_clone.path().join("README.md"), "# Registry").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();

        // Create the project
        let project_tmp = tempfile::tempdir().unwrap();
        setup_publish_project(project_tmp.path(), &bare_dir.display().to_string());

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "local-reg".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok(), "publish should succeed: {:?}", result.err());
    }

    #[test]
    fn test_run_invalid_version() {
        let project_tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            project_tmp.path().join("agent-environment.yml"),
            "name: test\ndescription: test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "official".to_string(),
            version: "not-semver".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid semver"));
    }

    #[test]
    fn test_run_no_manifest() {
        let empty_tmp = tempfile::tempdir().unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(empty_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "official".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no agent-environment.yml"));
    }

    #[test]
    fn test_run_no_description() {
        let project_tmp = tempfile::tempdir().unwrap();
        // Manifest without description
        std::fs::write(
            project_tmp.path().join("agent-environment.yml"),
            "name: test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "official".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("description"));
    }

    #[test]
    fn test_run_registry_not_found() {
        let project_tmp = tempfile::tempdir().unwrap();
        // init git so we can get past validate_manifest
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::fs::write(
            project_tmp.path().join("agent-environment.yml"),
            "name: test\ndescription: A test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "nonexistent-registry".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_run_dirty_working_tree() {
        let project_tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::fs::write(
            project_tmp.path().join("agent-environment.yml"),
            "name: test\ndescription: A test\nbackends:\n  claude-code: {}\nregistries:\n  myreg:\n    url: https://example.com/reg\n",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(project_tmp.path())
            .output()
            .unwrap();

        // Make the tree dirty
        std::fs::write(project_tmp.path().join("dirty.txt"), "dirty").unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "myreg".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("uncommitted changes"));
    }

    #[test]
    fn test_run_version_already_exists() {
        // Create a bare registry repo with marketplace.json already containing version 1.0.0
        let registry_tmp = tempfile::tempdir().unwrap();
        let bare_dir = registry_tmp.path().join("registry.git");
        std::fs::create_dir_all(&bare_dir).unwrap();
        create_bare_registry(&bare_dir);

        // Clone, add marketplace.json with existing version, push
        let init_clone = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", &bare_dir.display().to_string(), "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();

        let existing_index = r#"{
  "plugins": {
    "my-publish-test": {
      "description": "existing",
      "versions": {
        "1.0.0": {
          "commit": "old-sha"
        }
      }
    }
  }
}"#;
        std::fs::write(init_clone.path().join("marketplace.json"), existing_index).unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add existing"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();

        // Create the project
        let project_tmp = tempfile::tempdir().unwrap();
        setup_publish_project(project_tmp.path(), &bare_dir.display().to_string());

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        let args = PublishArgs {
            registry: "local-reg".to_string(),
            version: "1.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_commit_and_push_success() {
        // Create bare remote
        let remote_tmp = tempfile::tempdir().unwrap();
        create_bare_registry(remote_tmp.path());

        // Clone it to a local dir, add initial commit
        let local_tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", &remote_tmp.path().display().to_string(), "."])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();
        std::fs::write(local_tmp.path().join("marketplace.json"), "{}").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push"])
            .current_dir(local_tmp.path())
            .output()
            .unwrap();

        // Now modify marketplace.json and test commit_and_push
        std::fs::write(local_tmp.path().join("marketplace.json"), r#"{"plugins":{}}"#).unwrap();

        let result = commit_and_push(local_tmp.path(), "test-plugin", "2.0.0");
        assert!(result.is_ok(), "commit_and_push should succeed: {:?}", result.err());
    }

    #[test]
    fn test_run_publish_new_version_with_existing_marketplace() {
        // Test publishing a second version to a registry that already has marketplace.json
        let registry_tmp = tempfile::tempdir().unwrap();
        let bare_dir = registry_tmp.path().join("registry.git");
        std::fs::create_dir_all(&bare_dir).unwrap();
        create_bare_registry(&bare_dir);

        let init_clone = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", &bare_dir.display().to_string(), "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();

        // marketplace.json with version 1.0.0 already present
        let existing_index = r#"{
  "plugins": {
    "my-publish-test": {
      "description": "existing",
      "versions": {
        "1.0.0": { "commit": "old-sha" }
      }
    }
  }
}
"#;
        std::fs::write(init_clone.path().join("marketplace.json"), existing_index).unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push"])
            .current_dir(init_clone.path())
            .output()
            .unwrap();

        // Create the project
        let project_tmp = tempfile::tempdir().unwrap();
        setup_publish_project(project_tmp.path(), &bare_dir.display().to_string());

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(project_tmp.path()).unwrap();

        // Publish version 2.0.0 (different from existing 1.0.0)
        let args = PublishArgs {
            registry: "local-reg".to_string(),
            version: "2.0.0".to_string(),
            yes: true,
        };
        let result = run(args);
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok(), "publish 2.0.0 should succeed: {:?}", result.err());
    }

    #[test]
    fn test_verify_clean_working_tree_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // Initialize git repo with a commit
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .unwrap();
        std::fs::write(path.join("test.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();

        // Now modify a tracked file to make it dirty
        std::fs::write(path.join("test.txt"), "modified").unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        let result = verify_clean_working_tree();
        std::env::set_current_dir(original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("uncommitted changes"));
    }
}
