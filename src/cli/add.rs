//! `agx add` — add a dependency to the manifest.

use agx::config;
use agx::manifest;
use agx::manifest::types::{
    AgentDependency, McpServerConfig, PluginDependency, SkillDependency,
};
use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use std::collections::BTreeMap;

use super::install;
use super::lock;

#[derive(Debug, Args)]
pub struct AddArgs {
    #[command(subcommand)]
    pub asset_type: AddAssetType,
}

#[derive(Debug, Subcommand)]
pub enum AddAssetType {
    /// Add a plugin from a registry or git source
    Plugin {
        /// Plugin name
        name: String,
        /// Registry name
        #[arg(long)]
        registry: Option<String>,
        /// Version constraint
        #[arg(long)]
        version: Option<String>,
        /// Git repository URL
        #[arg(long)]
        git: Option<String>,
        /// Git tag
        #[arg(long)]
        tag: Option<String>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Git commit SHA
        #[arg(long)]
        commit: Option<String>,
    },
    /// Add a skill from a git source or local path
    Skill {
        /// Skill name
        name: String,
        /// Path (local or within repo)
        #[arg(long)]
        path: Option<String>,
        /// Git repository URL
        #[arg(long)]
        git: Option<String>,
        /// Git tag
        #[arg(long)]
        tag: Option<String>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Git commit SHA
        #[arg(long)]
        commit: Option<String>,
    },
    /// Add an agent from a git source or local path
    Agent {
        /// Agent name
        name: String,
        /// Path (local or within repo)
        #[arg(long)]
        path: Option<String>,
        /// Git repository URL
        #[arg(long)]
        git: Option<String>,
        /// Git tag
        #[arg(long)]
        tag: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
    },
    /// Add an MCP server
    Mcp {
        /// Server name
        name: String,
        /// SSE endpoint URL
        #[arg(long)]
        url: Option<String>,
        /// Executable command
        #[arg(long)]
        command: Option<String>,
    },
}

/// Run the `agx add` command.
///
/// Loads the manifest, adds the new dependency, saves the manifest,
/// and re-runs lock + install.
pub fn run(args: AddArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in {} or parent directories",
            config::MANIFEST_FILE,
            cwd.display()
        )
    })?;
    let project_root = manifest_path
        .parent()
        .expect("manifest must have a parent directory")
        .to_path_buf();

    let mut env = manifest::load(&manifest_path)?;

    match args.asset_type {
        AddAssetType::Plugin {
            name,
            registry,
            version,
            git,
            tag,
            branch,
            commit,
        } => {
            if registry.is_none() && git.is_none() {
                bail!("plugin requires either --registry or --git");
            }
            if registry.is_some() && version.is_none() {
                bail!("registry plugin requires --version");
            }
            let dep = PluginDependency {
                registry,
                version,
                git,
                tag,
                branch,
                commit,
                include: None,
                exclude: None,
            };
            eprintln!("adding plugin '{name}'...");
            env.plugins.insert(name, dep);
        }
        AddAssetType::Skill {
            name,
            path,
            git,
            tag,
            branch,
            commit,
        } => {
            let path = path.ok_or_else(|| anyhow::anyhow!("skill requires --path"))?;
            let dep = SkillDependency {
                git,
                path,
                tag,
                branch,
                commit,
            };
            eprintln!("adding skill '{name}'...");
            env.skills.insert(name, dep);
        }
        AddAssetType::Agent {
            name,
            path,
            git,
            tag,
            model,
        } => {
            let path = path.ok_or_else(|| anyhow::anyhow!("agent requires --path"))?;
            let dep = AgentDependency {
                git,
                path,
                tag,
                branch: None,
                commit: None,
                model,
                parameters: BTreeMap::new(),
            };
            eprintln!("adding agent '{name}'...");
            env.agents.insert(name, dep);
        }
        AddAssetType::Mcp {
            name,
            url,
            command,
        } => {
            if url.is_none() && command.is_none() {
                bail!("MCP server requires either --url or --command");
            }
            let config = McpServerConfig {
                url,
                command,
                args: vec![],
                env: BTreeMap::new(),
            };
            eprintln!("adding MCP server '{name}'...");
            env.mcp_servers.insert(name, config);
        }
    }

    // Save the updated manifest
    manifest::save(&env, &manifest_path)?;
    eprintln!("manifest updated.");

    // Re-run lock + install
    let lockfile = lock::run_lock(&env, &project_root)?;
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    agx::lockfile::save(&lockfile, &lockfile_path)?;

    // Run install (which will see the fresh lockfile)
    install::run(install::InstallArgs {})?;

    Ok(())
}
