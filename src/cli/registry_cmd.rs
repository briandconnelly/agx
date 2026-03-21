//! `agx registry` — registry management subcommands.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use console::style;

use agx::cache;
use agx::config;
use agx::manifest;
use agx::manifest::types::RegistryConfig;
use agx::sources::registry;

#[derive(Debug, Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    pub command: RegistryCommands,
}

#[derive(Debug, Subcommand)]
pub enum RegistryCommands {
    /// Add a marketplace registry
    Add {
        /// Registry name
        name: String,
        /// Registry git URL
        url: String,
        /// Authentication method (e.g. "token")
        #[arg(long)]
        auth: Option<String>,
    },
    /// Remove a registry
    Remove {
        /// Registry name
        name: String,
    },
    /// List configured registries
    List,
    /// Re-fetch registry index (all or one)
    Refresh {
        /// Specific registry to refresh (all if omitted)
        name: Option<String>,
    },
    /// Pin a registry to its current commit SHA
    Pin {
        /// Registry name
        name: String,
    },
}

pub fn run(args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommands::Add { name, url, auth } => run_add(&name, &url, auth.as_deref()),
        RegistryCommands::Remove { name } => run_remove(&name),
        RegistryCommands::List => run_list(),
        RegistryCommands::Refresh { name } => run_refresh(name.as_deref()),
        RegistryCommands::Pin { name } => run_pin(&name),
    }
}

/// Add a registry to the manifest.
fn run_add(name: &str, url: &str, auth: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let mut env = manifest::load(&manifest_path)?;

    if env.registries.contains_key(name) {
        bail!("registry '{}' already exists in manifest", name);
    }

    env.registries.insert(
        name.to_string(),
        RegistryConfig {
            url: url.to_string(),
            auth: auth.map(|s| s.to_string()),
            pin: None,
        },
    );

    manifest::save(&env, &manifest_path)?;
    println!(
        "{} Added registry '{}' ({})",
        style("✓").green(),
        style(name).bold(),
        url
    );

    Ok(())
}

/// Remove a registry from the manifest.
fn run_remove(name: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let mut env = manifest::load(&manifest_path)?;

    if env.registries.remove(name).is_none() {
        bail!("registry '{}' not found in manifest", name);
    }

    // Warn if any plugins reference this registry
    let referencing: Vec<&str> = env
        .plugins
        .iter()
        .filter(|(_, p)| p.registry.as_deref() == Some(name))
        .map(|(n, _)| n.as_str())
        .collect();

    if !referencing.is_empty() {
        eprintln!(
            "{} The following plugins still reference registry '{}':",
            style("warning:").yellow().bold(),
            name
        );
        for plugin_name in &referencing {
            eprintln!("  - {}", plugin_name);
        }
        eprintln!(
            "  These plugins will fail to resolve until you update their registry."
        );
    }

    manifest::save(&env, &manifest_path)?;
    println!(
        "{} Removed registry '{}'",
        style("✓").green(),
        style(name).bold()
    );

    Ok(())
}

/// List all configured registries.
fn run_list() -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let env = manifest::load(&manifest_path)?;

    if env.registries.is_empty() {
        println!("No registries configured.");
        return Ok(());
    }

    println!("{}", style("Registries:").bold());
    for (name, reg) in &env.registries {
        let auth_info = if let Some(ref auth) = reg.auth {
            let env_var = format!(
                "AGX_REGISTRY_{}_TOKEN",
                name.to_uppercase().replace('-', "_")
            );
            let configured = std::env::var(&env_var).is_ok();
            if configured {
                format!(" (auth: {auth}, {env_var} set)")
            } else {
                format!(
                    " (auth: {auth}, {} {})",
                    style(&env_var).yellow(),
                    style("not set").red()
                )
            }
        } else {
            String::new()
        };

        println!(
            "  {} {}{}",
            style(name).bold().cyan(),
            reg.url,
            auth_info
        );
    }

    Ok(())
}

/// Refresh registry indexes — fetch (or re-fetch) from git.
fn run_refresh(name: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let env = manifest::load(&manifest_path)?;

    let registries_to_refresh: Vec<(&str, &RegistryConfig)> = if let Some(name) = name {
        let reg = env
            .registries
            .get(name)
            .with_context(|| format!("registry '{}' not found in manifest", name))?;
        vec![(name, reg)]
    } else {
        env.registries.iter().map(|(n, r)| (n.as_str(), r)).collect()
    };

    if registries_to_refresh.is_empty() {
        println!("No registries configured.");
        return Ok(());
    }

    for (reg_name, reg_config) in &registries_to_refresh {
        let url = resolve_registry_url(reg_name, &reg_config.url, reg_config.auth.as_deref())?;
        let cache_path = cache::registry_dir()?.join(reg_name);

        print!("Refreshing registry '{}'... ", style(reg_name).bold());
        match registry::fetch_index(&url, &cache_path) {
            Ok(index) => {
                println!(
                    "{} ({} plugins)",
                    style("ok").green(),
                    index.plugins.len()
                );
            }
            Err(e) => {
                println!("{}", style("failed").red());
                eprintln!("  {}: {:#}", style("error").red().bold(), e);
            }
        }
    }

    Ok(())
}

/// Pin a registry to its current HEAD commit SHA.
fn run_pin(name: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let mut env = manifest::load(&manifest_path)?;

    let reg_config = env
        .registries
        .get(name)
        .with_context(|| format!("registry '{}' not found in manifest", name))?;

    let reg_cache = cache::registry_dir()?;
    let slug = agx::sources::git::url_to_slug(&reg_config.url);
    let reg_dir = reg_cache.join(&slug);

    if !reg_dir.exists() {
        bail!(
            "registry '{}' is not cached; run `agx registry refresh {}` first",
            name,
            name
        );
    }

    // Get the current HEAD commit SHA
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&reg_dir)
        .output()
        .context("reading HEAD commit from cached registry")?;

    if !output.status.success() {
        bail!(
            "failed to read HEAD commit from cached registry '{}'",
            name
        );
    }

    let sha = String::from_utf8(output.stdout)
        .context("invalid UTF-8 in git output")?
        .trim()
        .to_string();

    // Update the manifest
    let reg = env
        .registries
        .get_mut(name)
        .expect("registry must exist (checked above)");
    reg.pin = Some(sha.clone());

    manifest::save(&env, &manifest_path)?;
    println!(
        "{} Pinned registry '{}' to {}",
        style("✓").green(),
        style(name).bold(),
        style(&sha).dim()
    );

    Ok(())
}

/// Resolve a registry URL, embedding an auth token if needed.
///
/// For registries with `auth: token`, checks for
/// `AGX_REGISTRY_{UPPER_SNAKE_NAME}_TOKEN` env var and embeds it in the
/// HTTPS URL for git authentication.
fn resolve_registry_url(name: &str, url: &str, auth: Option<&str>) -> Result<String> {
    if auth != Some("token") {
        return Ok(url.to_string());
    }

    let env_var = format!(
        "AGX_REGISTRY_{}_TOKEN",
        name.to_uppercase().replace('-', "_")
    );

    let token = std::env::var(&env_var).with_context(|| {
        format!(
            "registry '{}' requires auth but {} is not set.\n\
             Set this environment variable to your access token:\n\
             \n  export {}=<your-token>\n",
            name, env_var, env_var
        )
    })?;

    // Embed token in HTTPS URL: https://TOKEN@github.com/...
    if let Some(rest) = url.strip_prefix("https://") {
        Ok(format!("https://{}@{}", token, rest))
    } else if let Some(rest) = url.strip_prefix("http://") {
        Ok(format!("http://{}@{}", token, rest))
    } else {
        // For non-HTTP URLs (SSH, etc.), we can't embed the token.
        // Fall back to the original URL and let git handle auth.
        eprintln!(
            "{} Cannot embed token in non-HTTPS URL '{}'; relying on git credentials.",
            style("warning:").yellow().bold(),
            url
        );
        Ok(url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn make_manifest(registries: BTreeMap<String, RegistryConfig>) -> AgentEnvironment {
        AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
            registries,
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        }
    }

    #[test]
    fn registry_add_to_manifest() {
        let mut env = make_manifest(BTreeMap::new());
        assert!(env.registries.is_empty());

        env.registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://github.com/anthropics/claude-plugins-official".to_string(),
                auth: None,
                pin: None,
            },
        );

        assert_eq!(env.registries.len(), 1);
        assert_eq!(
            env.registries["official"].url,
            "https://github.com/anthropics/claude-plugins-official"
        );
    }

    #[test]
    fn registry_remove_from_manifest() {
        let mut registries = BTreeMap::new();
        registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://github.com/anthropics/claude-plugins-official".to_string(),
                auth: None,
                pin: None,
            },
        );
        let mut env = make_manifest(registries);
        assert_eq!(env.registries.len(), 1);

        env.registries.remove("official");
        assert!(env.registries.is_empty());
    }

    #[test]
    fn registry_remove_warns_on_referencing_plugins() {
        let mut registries = BTreeMap::new();
        registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://example.com".to_string(),
                auth: None,
                pin: None,
            },
        );
        let mut env = make_manifest(registries);
        env.plugins.insert(
            "quality-review".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=1.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );

        // After removing registry, plugin still references it
        let referencing: Vec<&str> = env
            .plugins
            .iter()
            .filter(|(_, p)| p.registry.as_deref() == Some("official"))
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(referencing, vec!["quality-review"]);
    }

    #[test]
    fn resolve_url_no_auth() {
        let url = resolve_registry_url("test", "https://github.com/org/repo", None).unwrap();
        assert_eq!(url, "https://github.com/org/repo");
    }

    #[test]
    fn resolve_url_with_token() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Safety: holding ENV_LOCK
        unsafe { std::env::set_var("AGX_REGISTRY_MY_REG_TOKEN", "ghp_abc123") };

        let url =
            resolve_registry_url("my-reg", "https://github.com/org/repo", Some("token")).unwrap();
        assert_eq!(url, "https://ghp_abc123@github.com/org/repo");

        unsafe { std::env::remove_var("AGX_REGISTRY_MY_REG_TOKEN") };
    }

    #[test]
    fn resolve_url_missing_token_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure the var is not set
        unsafe { std::env::remove_var("AGX_REGISTRY_MISSING_TOKEN") };

        let result =
            resolve_registry_url("missing", "https://github.com/org/repo", Some("token"));
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("AGX_REGISTRY_MISSING_TOKEN"));
    }

    #[test]
    fn env_var_name_with_hyphens() {
        // Hyphens in registry name should be converted to underscores
        let env_var = format!(
            "AGX_REGISTRY_{}_TOKEN",
            "my-private-reg".to_uppercase().replace('-', "_")
        );
        assert_eq!(env_var, "AGX_REGISTRY_MY_PRIVATE_REG_TOKEN");
    }

    // --- resolve_registry_url edge cases ---

    /// Non-token auth type should pass URL through unchanged.
    #[test]
    fn resolve_url_non_token_auth() {
        let url =
            resolve_registry_url("test", "https://github.com/org/repo", Some("oauth")).unwrap();
        assert_eq!(url, "https://github.com/org/repo");
    }

    /// HTTP URL (not HTTPS) with token auth should embed token.
    #[test]
    fn resolve_url_http_with_token() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AGX_REGISTRY_HTTP_REG_TOKEN", "tok_xyz") };

        let url = resolve_registry_url(
            "http-reg",
            "http://internal.example.com/registry",
            Some("token"),
        )
        .unwrap();
        assert_eq!(url, "http://tok_xyz@internal.example.com/registry");

        unsafe { std::env::remove_var("AGX_REGISTRY_HTTP_REG_TOKEN") };
    }

    /// SSH URL with token auth should pass through with a warning (can't embed token).
    #[test]
    fn resolve_url_ssh_with_token_passthrough() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AGX_REGISTRY_SSH_REG_TOKEN", "tok_ssh") };

        let url = resolve_registry_url(
            "ssh-reg",
            "git@github.com:org/repo.git",
            Some("token"),
        )
        .unwrap();
        // SSH URLs can't embed tokens, so it passes through
        assert_eq!(url, "git@github.com:org/repo.git");

        unsafe { std::env::remove_var("AGX_REGISTRY_SSH_REG_TOKEN") };
    }

    /// Token with special characters should be embedded correctly.
    #[test]
    fn resolve_url_token_with_special_chars() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AGX_REGISTRY_SPECIAL_TOKEN", "ghp_abc/123+def") };

        let url = resolve_registry_url(
            "special",
            "https://github.com/org/repo",
            Some("token"),
        )
        .unwrap();
        assert_eq!(url, "https://ghp_abc/123+def@github.com/org/repo");

        unsafe { std::env::remove_var("AGX_REGISTRY_SPECIAL_TOKEN") };
    }

    // --- run_add / run_remove / run_list via manifest file ---

    /// Helper: write a manifest to a tempdir and return the path to it.
    fn write_manifest_to(dir: &std::path::Path, manifest: &AgentEnvironment) {
        let path = dir.join(config::MANIFEST_FILE);
        manifest::save(manifest, &path).unwrap();
    }

    /// Test `run_add` adds a registry to the manifest file.
    #[test]
    fn run_add_creates_registry_entry() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        // run_add uses current_dir(), so we need to set it
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_add("my-reg", "https://github.com/org/registry.git", None);
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        // Verify the manifest was updated
        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert_eq!(loaded.registries.len(), 1);
        assert_eq!(
            loaded.registries["my-reg"].url,
            "https://github.com/org/registry.git"
        );
        assert!(loaded.registries["my-reg"].auth.is_none());
    }

    /// Test `run_add` with auth parameter.
    #[test]
    fn run_add_with_auth() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_add(
            "private-reg",
            "https://github.com/org/private-registry.git",
            Some("token"),
        );
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert_eq!(loaded.registries["private-reg"].auth.as_deref(), Some("token"));
    }

    /// Test `run_add` with duplicate registry name errors.
    #[test]
    fn run_add_duplicate_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let mut registries = BTreeMap::new();
        registries.insert(
            "existing".to_string(),
            RegistryConfig {
                url: "https://example.com/reg.git".to_string(),
                auth: None,
                pin: None,
            },
        );
        let manifest = make_manifest(registries);
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_add("existing", "https://other.com/reg.git", None);
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("already exists"));
    }

    /// Test `run_remove` removes a registry from the manifest file.
    #[test]
    fn run_remove_existing_registry() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let mut registries = BTreeMap::new();
        registries.insert(
            "to-remove".to_string(),
            RegistryConfig {
                url: "https://example.com/reg.git".to_string(),
                auth: None,
                pin: None,
            },
        );
        let manifest = make_manifest(registries);
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_remove("to-remove");
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert!(loaded.registries.is_empty());
    }

    /// Test `run_remove` with nonexistent registry errors.
    #[test]
    fn run_remove_nonexistent_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_remove("nonexistent");
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("not found"));
    }

    /// Test `run_remove` warns when plugins reference the removed registry.
    #[test]
    fn run_remove_warns_referencing_plugins() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let mut registries = BTreeMap::new();
        registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://example.com/reg.git".to_string(),
                auth: None,
                pin: None,
            },
        );
        let mut manifest = make_manifest(registries);
        manifest.plugins.insert(
            "some-plugin".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=1.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Should succeed but print a warning (we can't easily capture stderr in a unit test,
        // but the function should still complete successfully)
        let result = run_remove("official");
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert!(loaded.registries.is_empty());
        // Plugin still references the removed registry
        assert_eq!(
            loaded.plugins["some-plugin"].registry.as_deref(),
            Some("official")
        );
    }

    /// Test `run_list` with empty registries.
    #[test]
    fn run_list_empty() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_list();
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();
    }

    /// Test `run_list` with populated registries.
    #[test]
    fn run_list_populated() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let mut registries = BTreeMap::new();
        registries.insert(
            "official".to_string(),
            RegistryConfig {
                url: "https://github.com/anthropics/registry.git".to_string(),
                auth: None,
                pin: None,
            },
        );
        registries.insert(
            "private".to_string(),
            RegistryConfig {
                url: "https://github.com/corp/private-registry.git".to_string(),
                auth: Some("token".to_string()),
                pin: None,
            },
        );
        let manifest = make_manifest(registries);
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_list();
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();
    }

    /// Test `run_list` with auth and env var set.
    #[test]
    fn run_list_with_auth_env_set() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let mut registries = BTreeMap::new();
        registries.insert(
            "auth-reg".to_string(),
            RegistryConfig {
                url: "https://github.com/org/registry.git".to_string(),
                auth: Some("token".to_string()),
                pin: None,
            },
        );
        let manifest = make_manifest(registries);
        write_manifest_to(tmp.path(), &manifest);

        unsafe { std::env::set_var("AGX_REGISTRY_AUTH_REG_TOKEN", "secret") };

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_list();
        std::env::set_current_dir(&original_dir).unwrap();

        unsafe { std::env::remove_var("AGX_REGISTRY_AUTH_REG_TOKEN") };

        result.unwrap();
    }

    /// Test `run_refresh` with no manifest errors.
    #[test]
    fn run_refresh_no_manifest_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // No manifest file written

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_refresh(None);
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
    }

    /// Test `run_refresh` with empty registries succeeds.
    #[test]
    fn run_refresh_empty_registries() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_refresh(None);
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();
    }

    /// Test `run_refresh` with a specific nonexistent registry name errors.
    #[test]
    fn run_refresh_nonexistent_registry_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_refresh(Some("nonexistent"));
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("not found"));
    }

    /// Test `run_pin` with no manifest errors.
    #[test]
    fn run_pin_no_manifest_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_pin("some-reg");
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
    }

    /// Test `run_pin` with nonexistent registry errors.
    #[test]
    fn run_pin_nonexistent_registry_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_pin("nonexistent");
        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("not found"));
    }

    /// Test `run_pin` with uncached registry errors.
    #[test]
    fn run_pin_uncached_registry_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        let mut registries = BTreeMap::new();
        registries.insert(
            "uncached".to_string(),
            RegistryConfig {
                url: "https://github.com/org/uncached-registry.git".to_string(),
                auth: None,
                pin: None,
            },
        );
        let manifest = make_manifest(registries);
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = run_pin("uncached");
        std::env::set_current_dir(&original_dir).unwrap();

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("not cached"));
    }

    /// Test `run_add` and then `run_list` in sequence.
    #[test]
    fn run_add_then_list() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        run_add("first", "https://example.com/first.git", None).unwrap();
        run_add("second", "https://example.com/second.git", Some("token")).unwrap();
        let result = run_list();

        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert_eq!(loaded.registries.len(), 2);
    }

    /// Test `run_add` then `run_remove` then verify manifest.
    #[test]
    fn run_add_then_remove() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let manifest = make_manifest(BTreeMap::new());
        write_manifest_to(tmp.path(), &manifest);

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        run_add("temp-reg", "https://example.com/temp.git", None).unwrap();

        // Verify it was added
        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert_eq!(loaded.registries.len(), 1);

        run_remove("temp-reg").unwrap();

        std::env::set_current_dir(&original_dir).unwrap();

        // Verify it was removed
        let loaded = manifest::load(&tmp.path().join(config::MANIFEST_FILE)).unwrap();
        assert!(loaded.registries.is_empty());
    }
}
