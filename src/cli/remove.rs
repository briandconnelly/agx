//! `agx remove` — remove a dependency from the manifest.

use agx::config;
use agx::manifest;
use anyhow::Context;
use clap::Args;

use super::install;
use super::lock;

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Name of the asset to remove
    pub name: String,
}

/// Run the `agx remove` command.
///
/// Loads the manifest, removes the named dependency from whichever
/// section it's in, saves the manifest, and re-runs lock + install.
pub fn run(args: RemoveArgs) -> anyhow::Result<()> {
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

    remove_from_manifest(&mut env, &args.name)?;

    // Save the updated manifest
    manifest::save(&env, &manifest_path)?;
    eprintln!("manifest updated.");

    // Re-run lock + install
    let lockfile = lock::run_lock(&env, &project_root)?;
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    agx::lockfile::save(&lockfile, &lockfile_path)?;

    install::run(install::InstallArgs {})?;

    Ok(())
}

/// Remove an asset by name from the manifest.
///
/// Searches plugins, skills, agents, and mcp-servers. Returns an error
/// if the name is not found in any section.
pub fn remove_from_manifest(
    env: &mut agx::manifest::types::AgentEnvironment,
    name: &str,
) -> anyhow::Result<()> {
    let mut found = false;

    if env.plugins.remove(name).is_some() {
        eprintln!("removed plugin '{name}'");
        found = true;
    }
    if env.skills.remove(name).is_some() {
        eprintln!("removed skill '{name}'");
        found = true;
    }
    if env.agents.remove(name).is_some() {
        eprintln!("removed agent '{name}'");
        found = true;
    }
    if env.mcp_servers.remove(name).is_some() {
        eprintln!("removed MCP server '{name}'");
        found = true;
    }

    if !found {
        anyhow::bail!(
            "'{name}' not found in plugins, skills, agents, or mcp-servers"
        );
    }

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
        }
    }

    #[test]
    fn remove_plugin() {
        let mut env = minimal_env();
        env.plugins.insert(
            "my-plugin".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some("^1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        assert_eq!(env.plugins.len(), 1);
        remove_from_manifest(&mut env, "my-plugin").unwrap();
        assert!(env.plugins.is_empty());
    }

    #[test]
    fn remove_skill() {
        let mut env = minimal_env();
        env.skills.insert(
            "deploy".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/deploy".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        remove_from_manifest(&mut env, "deploy").unwrap();
        assert!(env.skills.is_empty());
    }

    #[test]
    fn remove_agent() {
        let mut env = minimal_env();
        env.agents.insert(
            "reviewer".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/reviewer.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        remove_from_manifest(&mut env, "reviewer").unwrap();
        assert!(env.agents.is_empty());
    }

    #[test]
    fn remove_mcp_server() {
        let mut env = minimal_env();
        env.mcp_servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        remove_from_manifest(&mut env, "playwright").unwrap();
        assert!(env.mcp_servers.is_empty());
    }

    #[test]
    fn remove_nonexistent_errors() {
        let mut env = minimal_env();
        let result = remove_from_manifest(&mut env, "does-not-exist");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "expected 'not found' error, got: {msg}"
        );
    }

    #[test]
    fn remove_leaves_other_entries_intact() {
        let mut env = minimal_env();
        env.plugins.insert(
            "keep-me".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.plugins.insert(
            "remove-me".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        remove_from_manifest(&mut env, "remove-me").unwrap();
        assert_eq!(env.plugins.len(), 1);
        assert!(env.plugins.contains_key("keep-me"));
    }

    #[test]
    fn remove_same_name_across_sections() {
        // If a name appears in multiple sections (unusual but possible),
        // all instances should be removed.
        let mut env = minimal_env();
        env.plugins.insert(
            "shared-name".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.skills.insert(
            "shared-name".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/shared-name".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        remove_from_manifest(&mut env, "shared-name").unwrap();
        assert!(env.plugins.is_empty());
        assert!(env.skills.is_empty());
    }

    #[test]
    fn remove_from_multiple_entries_same_type() {
        let mut env = minimal_env();
        for name in &["plugin-a", "plugin-b", "plugin-c"] {
            env.plugins.insert(
                name.to_string(),
                PluginDependency {
                    registry: Some("r".to_string()),
                    version: Some("^1".to_string()),
                    git: None,
                    tag: None,
                    branch: None,
                    commit: None,
                    include: None,
                    exclude: None,
                },
            );
        }
        assert_eq!(env.plugins.len(), 3);
        remove_from_manifest(&mut env, "plugin-b").unwrap();
        assert_eq!(env.plugins.len(), 2);
        assert!(env.plugins.contains_key("plugin-a"));
        assert!(!env.plugins.contains_key("plugin-b"));
        assert!(env.plugins.contains_key("plugin-c"));
    }

    #[test]
    fn remove_nonexistent_error_message_contains_name() {
        let mut env = minimal_env();
        let err = remove_from_manifest(&mut env, "ghost-dep").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ghost-dep"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn run_no_manifest_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(RemoveArgs {
            name: "anything".to_string(),
        });
        std::env::set_current_dir(&old_cwd).unwrap();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no agent-environment.yml"));
    }

    #[test]
    fn run_with_manifest_but_missing_dep_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("agent-environment.yml");
        std::fs::write(
            &manifest_path,
            "name: test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(RemoveArgs {
            name: "nonexistent".to_string(),
        });
        std::env::set_current_dir(&old_cwd).unwrap();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found"));
    }

    #[test]
    fn remove_agent_leaves_skills_and_plugins() {
        let mut env = minimal_env();
        env.plugins.insert(
            "my-plugin".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.skills.insert(
            "my-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/my-skill".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.agents.insert(
            "my-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/my-agent.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        remove_from_manifest(&mut env, "my-agent").unwrap();
        assert!(env.agents.is_empty());
        assert_eq!(env.plugins.len(), 1);
        assert_eq!(env.skills.len(), 1);
    }
}
