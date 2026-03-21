//! `agx which` — show the resolved source and file path of an asset.

use anyhow::{bail, Context, Result};
use clap::Args;

use agx::cache;
use agx::config;
use agx::lockfile;
use agx::lockfile::types::{LockedSource, Lockfile};
use agx::manifest;

#[derive(Debug, Args)]
pub struct WhichArgs {
    /// Asset name
    pub name: String,
}

pub fn run(args: WhichArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let project_dir = manifest_path.parent().unwrap();
    let manifest = manifest::load(&manifest_path)?;

    // Try to load lockfile
    let lockfile_path = project_dir.join(config::LOCKFILE_FILE);
    let lockfile = if lockfile_path.exists() {
        Some(lockfile::load(&lockfile_path)?)
    } else {
        None
    };

    let name = &args.name;

    if let Some(plugin) = manifest.plugins.get(name) {
        print_plugin_path(name, plugin, lockfile.as_ref())?;
    } else if let Some(skill) = manifest.skills.get(name) {
        print_skill_path(name, skill, lockfile.as_ref())?;
    } else if let Some(agent) = manifest.agents.get(name) {
        print_agent_path(name, agent, lockfile.as_ref())?;
    } else {
        bail!("asset '{}' not found in manifest", name);
    }

    Ok(())
}

/// Print the resolved path for a plugin.
fn print_plugin_path(
    name: &str,
    _plugin: &agx::manifest::types::PluginDependency,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    if let Some(resolved) = lockfile.and_then(|lf| lf.plugins.get(name)) {
        let cache_key = match &resolved.source {
            LockedSource::Registry { resolved_version, .. } => resolved_version.clone(),
            _ => resolved.git_commit.clone().unwrap_or_default(),
        };
        if let Ok(Some(path)) = cache::get_plugin(name, &cache_key) {
            println!("{}", path.display());
            return Ok(());
        }
        // Not cached yet — show where it would be
        let expected = cache::plugins_dir()?.join(name).join(&cache_key);
        println!("{}", expected.display());
    } else {
        // No lockfile entry — show expected cache location
        let expected = cache::plugins_dir()?.join(name);
        println!("{}/", expected.display());
    }
    Ok(())
}

/// Print the resolved path for a skill.
fn print_skill_path(
    name: &str,
    skill: &agx::manifest::types::SkillDependency,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    if skill.is_local() {
        println!("{}", skill.path);
        return Ok(());
    }

    if let Some(resolved) = lockfile.and_then(|lf| lf.skills.get(name)) {
        if let Some(commit) = resolved.git_commit.as_deref() {
            if let Ok(Some(path)) = cache::get_skill(name, commit) {
                println!("{}", path.display());
                return Ok(());
            }
            // Not cached yet
            let expected = cache::skills_dir()?.join(name).join(commit);
            println!("{}", expected.display());
            return Ok(());
        }
    }

    // Fallback
    let expected = cache::skills_dir()?.join(name);
    println!("{}/", expected.display());
    Ok(())
}

/// Print the resolved path for an agent.
fn print_agent_path(
    name: &str,
    agent: &agx::manifest::types::AgentDependency,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    if agent.is_local() {
        println!("{}", agent.path);
        return Ok(());
    }

    if let Some(resolved) = lockfile.and_then(|lf| lf.agents.get(name)) {
        if let Some(commit) = resolved.git_commit.as_deref() {
            if let Ok(Some(path)) = cache::get_agent(name, commit) {
                println!("{}", path.display());
                return Ok(());
            }
            // Not cached yet
            let expected = cache::agents_dir()?.join(name).join(commit);
            println!("{}", expected.display());
            return Ok(());
        }
    }

    // Fallback
    let expected = cache::agents_dir()?.join(name);
    println!("{}/", expected.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use agx::manifest::types::*;
    use agx::lockfile::types::*;

    #[test]
    fn local_skill_path() {
        let skill = SkillDependency {
            git: None,
            path: "./skills/deploy-checklist".to_string(),
            tag: None,
            branch: None,
            commit: None,
        };
        assert!(skill.is_local());
        assert_eq!(skill.path, "./skills/deploy-checklist");
    }

    #[test]
    fn local_agent_path() {
        let agent = AgentDependency {
            git: None,
            path: "./agents/my-reviewer.md".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: Some("opus".to_string()),
            parameters: Default::default(),
        };
        assert!(agent.is_local());
        assert_eq!(agent.path, "./agents/my-reviewer.md");
    }

    #[test]
    fn cached_plugin_lookup_key_from_registry() {
        let source = LockedSource::Registry {
            registry: "official".to_string(),
            resolved_version: "1.2.0".to_string(),
            path: None,
        };
        match &source {
            LockedSource::Registry { resolved_version, .. } => {
                assert_eq!(resolved_version, "1.2.0");
            }
            _ => panic!("expected registry source"),
        }
    }

    #[test]
    fn cached_plugin_lookup_key_from_git() {
        let locked = LockedPlugin {
            source: LockedSource::Git {
                git: "https://github.com/someone/plugin.git".to_string(),
                path: None,
            },
            git_commit: Some("abc123def456".to_string()),
            integrity: None,
        };
        assert_eq!(locked.git_commit.as_deref(), Some("abc123def456"));
    }

    #[test]
    fn remote_skill_not_local() {
        let skill = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        assert!(!skill.is_local());
    }

    #[test]
    fn remote_agent_not_local() {
        let agent = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/helper.md".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        assert!(!agent.is_local());
    }
}
