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
}

pub fn run(args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommands::Add { name, url, auth } => run_add(&name, &url, auth.as_deref()),
        RegistryCommands::Remove { name } => run_remove(&name),
        RegistryCommands::List => run_list(),
        RegistryCommands::Refresh { name } => run_refresh(name.as_deref()),
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
    use std::sync::Mutex;

    /// Global lock for tests that modify env vars.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let _guard = ENV_LOCK.lock().unwrap();
        // Safety: holding ENV_LOCK
        unsafe { std::env::set_var("AGX_REGISTRY_MY_REG_TOKEN", "ghp_abc123") };

        let url =
            resolve_registry_url("my-reg", "https://github.com/org/repo", Some("token")).unwrap();
        assert_eq!(url, "https://ghp_abc123@github.com/org/repo");

        unsafe { std::env::remove_var("AGX_REGISTRY_MY_REG_TOKEN") };
    }

    #[test]
    fn resolve_url_missing_token_errors() {
        let _guard = ENV_LOCK.lock().unwrap();
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
}
