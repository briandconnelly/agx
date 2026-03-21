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

/// Add an asset to the manifest based on the parsed CLI arguments.
///
/// This is the core mutation logic, separated from I/O (loading/saving manifest,
/// running lock + install) so it can be unit-tested.
pub fn add_to_manifest(
    env: &mut agx::manifest::types::AgentEnvironment,
    asset_type: AddAssetType,
) -> anyhow::Result<()> {
    match asset_type {
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
    Ok(())
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

    add_to_manifest(&mut env, args.asset_type)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn minimal_env() -> AgentEnvironment {
        AgentEnvironment {
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
            env_file: None,
        }
    }

    #[test]
    fn add_plugin_with_registry() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "my-plugin".to_string(),
            registry: Some("official".to_string()),
            version: Some("^1.0.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        assert_eq!(env.plugins.len(), 1);
        let dep = &env.plugins["my-plugin"];
        assert_eq!(dep.registry.as_deref(), Some("official"));
        assert_eq!(dep.version.as_deref(), Some("^1.0.0"));
    }

    #[test]
    fn add_plugin_with_git() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "git-plugin".to_string(),
            registry: None,
            version: None,
            git: Some("https://github.com/org/repo.git".to_string()),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        assert_eq!(env.plugins.len(), 1);
        let dep = &env.plugins["git-plugin"];
        assert!(dep.git.is_some());
        assert_eq!(dep.tag.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn add_plugin_with_git_branch() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "branch-plugin".to_string(),
            registry: None,
            version: None,
            git: Some("https://github.com/org/repo.git".to_string()),
            tag: None,
            branch: Some("main".to_string()),
            commit: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        let dep = &env.plugins["branch-plugin"];
        assert_eq!(dep.branch.as_deref(), Some("main"));
    }

    #[test]
    fn add_plugin_with_git_commit() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "commit-plugin".to_string(),
            registry: None,
            version: None,
            git: Some("https://github.com/org/repo.git".to_string()),
            tag: None,
            branch: None,
            commit: Some("abc123def456".to_string()),
        };
        add_to_manifest(&mut env, asset).unwrap();
        let dep = &env.plugins["commit-plugin"];
        assert_eq!(dep.commit.as_deref(), Some("abc123def456"));
    }

    #[test]
    fn add_plugin_requires_registry_or_git() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "bad-plugin".to_string(),
            registry: None,
            version: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        let result = add_to_manifest(&mut env, asset);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("plugin requires either --registry or --git"));
    }

    #[test]
    fn add_plugin_registry_requires_version() {
        let mut env = minimal_env();
        let asset = AddAssetType::Plugin {
            name: "no-ver".to_string(),
            registry: Some("official".to_string()),
            version: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        let result = add_to_manifest(&mut env, asset);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("registry plugin requires --version"));
    }

    #[test]
    fn add_skill_with_local_path() {
        let mut env = minimal_env();
        let asset = AddAssetType::Skill {
            name: "deploy".to_string(),
            path: Some("./skills/deploy".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        assert_eq!(env.skills.len(), 1);
        assert_eq!(env.skills["deploy"].path, "./skills/deploy");
        assert!(env.skills["deploy"].git.is_none());
    }

    #[test]
    fn add_skill_with_git() {
        let mut env = minimal_env();
        let asset = AddAssetType::Skill {
            name: "remote-skill".to_string(),
            path: Some("skills/review".to_string()),
            git: Some("https://github.com/org/repo.git".to_string()),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        let dep = &env.skills["remote-skill"];
        assert_eq!(dep.git.as_deref(), Some("https://github.com/org/repo.git"));
        assert_eq!(dep.tag.as_deref(), Some("v2.0.0"));
    }

    #[test]
    fn add_skill_requires_path() {
        let mut env = minimal_env();
        let asset = AddAssetType::Skill {
            name: "no-path".to_string(),
            path: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        let result = add_to_manifest(&mut env, asset);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("skill requires --path"));
    }

    #[test]
    fn add_agent_with_local_path() {
        let mut env = minimal_env();
        let asset = AddAssetType::Agent {
            name: "reviewer".to_string(),
            path: Some("./agents/reviewer.md".to_string()),
            git: None,
            tag: None,
            model: Some("opus".to_string()),
        };
        add_to_manifest(&mut env, asset).unwrap();
        assert_eq!(env.agents.len(), 1);
        let dep = &env.agents["reviewer"];
        assert_eq!(dep.path, "./agents/reviewer.md");
        assert_eq!(dep.model.as_deref(), Some("opus"));
    }

    #[test]
    fn add_agent_with_git() {
        let mut env = minimal_env();
        let asset = AddAssetType::Agent {
            name: "remote-agent".to_string(),
            path: Some("agents/helper.md".to_string()),
            git: Some("https://github.com/org/agents.git".to_string()),
            tag: Some("v1.0.0".to_string()),
            model: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        let dep = &env.agents["remote-agent"];
        assert!(dep.git.is_some());
        assert_eq!(dep.tag.as_deref(), Some("v1.0.0"));
        assert!(dep.branch.is_none());
        assert!(dep.commit.is_none());
    }

    #[test]
    fn add_agent_requires_path() {
        let mut env = minimal_env();
        let asset = AddAssetType::Agent {
            name: "no-path".to_string(),
            path: None,
            git: None,
            tag: None,
            model: None,
        };
        let result = add_to_manifest(&mut env, asset);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("agent requires --path"));
    }

    #[test]
    fn add_mcp_server_with_url() {
        let mut env = minimal_env();
        let asset = AddAssetType::Mcp {
            name: "playwright".to_string(),
            url: Some("https://mcp.playwright.dev/sse".to_string()),
            command: None,
        };
        add_to_manifest(&mut env, asset).unwrap();
        assert_eq!(env.mcp_servers.len(), 1);
        let cfg = &env.mcp_servers["playwright"];
        assert_eq!(cfg.url.as_deref(), Some("https://mcp.playwright.dev/sse"));
        assert!(cfg.command.is_none());
    }

    #[test]
    fn add_mcp_server_with_command() {
        let mut env = minimal_env();
        let asset = AddAssetType::Mcp {
            name: "filesystem".to_string(),
            url: None,
            command: Some("npx".to_string()),
        };
        add_to_manifest(&mut env, asset).unwrap();
        let cfg = &env.mcp_servers["filesystem"];
        assert_eq!(cfg.command.as_deref(), Some("npx"));
        assert!(cfg.url.is_none());
        assert!(cfg.args.is_empty());
        assert!(cfg.env.is_empty());
    }

    #[test]
    fn add_mcp_server_requires_url_or_command() {
        let mut env = minimal_env();
        let asset = AddAssetType::Mcp {
            name: "broken".to_string(),
            url: None,
            command: None,
        };
        let result = add_to_manifest(&mut env, asset);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("MCP server requires either --url or --command"));
    }

    #[test]
    fn add_plugin_overwrites_existing() {
        let mut env = minimal_env();
        // Add first version
        let asset1 = AddAssetType::Plugin {
            name: "my-plugin".to_string(),
            registry: Some("official".to_string()),
            version: Some("^1.0.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset1).unwrap();
        assert_eq!(env.plugins["my-plugin"].version.as_deref(), Some("^1.0.0"));

        // Add same name with different version — overwrites
        let asset2 = AddAssetType::Plugin {
            name: "my-plugin".to_string(),
            registry: Some("official".to_string()),
            version: Some("^2.0.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
        };
        add_to_manifest(&mut env, asset2).unwrap();
        assert_eq!(env.plugins.len(), 1);
        assert_eq!(env.plugins["my-plugin"].version.as_deref(), Some("^2.0.0"));
    }

    #[test]
    fn add_multiple_different_asset_types() {
        let mut env = minimal_env();

        add_to_manifest(
            &mut env,
            AddAssetType::Plugin {
                name: "p1".to_string(),
                registry: Some("r".to_string()),
                version: Some("^1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
            },
        )
        .unwrap();

        add_to_manifest(
            &mut env,
            AddAssetType::Skill {
                name: "s1".to_string(),
                path: Some("./skills/s1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
            },
        )
        .unwrap();

        add_to_manifest(
            &mut env,
            AddAssetType::Agent {
                name: "a1".to_string(),
                path: Some("./agents/a1.md".to_string()),
                git: None,
                tag: None,
                model: None,
            },
        )
        .unwrap();

        add_to_manifest(
            &mut env,
            AddAssetType::Mcp {
                name: "m1".to_string(),
                url: Some("https://example.com/sse".to_string()),
                command: None,
            },
        )
        .unwrap();

        assert_eq!(env.plugins.len(), 1);
        assert_eq!(env.skills.len(), 1);
        assert_eq!(env.agents.len(), 1);
        assert_eq!(env.mcp_servers.len(), 1);
    }
}
