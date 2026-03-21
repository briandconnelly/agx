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
    let project_dir = manifest_path.parent().expect("manifest must have a parent directory");
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

    // ---- Tests that exercise print_*_path functions for line coverage ----

    use super::*;

    #[test]
    fn print_plugin_path_registry_lockfile() {
        let plugin = PluginDependency {
            registry: Some("official".to_string()),
            version: Some(">=1.0.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "my-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    resolved_version: "1.2.0".to_string(),
                    path: None,
                },
                git_commit: Some("a1b2c3d4".to_string()),
                integrity: None,
            },
        );
        // This will print a path (cached or expected); just verify no error
        print_plugin_path("my-plugin", &plugin, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_plugin_path_git_lockfile() {
        let plugin = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "git-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("deadbeef12345678".to_string()),
                integrity: None,
            },
        );
        print_plugin_path("git-plugin", &plugin, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_plugin_path_no_lockfile() {
        let plugin = PluginDependency {
            registry: None,
            version: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        print_plugin_path("unlocked-plugin", &plugin, None).unwrap();
    }

    #[test]
    fn print_skill_path_local() {
        let skill = SkillDependency {
            git: None,
            path: "./skills/my-skill".to_string(),
            tag: None,
            branch: None,
            commit: None,
        };
        print_skill_path("my-skill", &skill, None).unwrap();
    }

    #[test]
    fn print_skill_path_remote_with_lockfile() {
        let skill = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        let mut lockfile = Lockfile::new();
        lockfile.skills.insert(
            "remote-skill".to_string(),
            LockedSkill {
                source: LockedSource::Git {
                    git: "https://github.com/someone/skills.git".to_string(),
                    path: None,
                },
                git_commit: Some("aabbccdd11223344".to_string()),
                integrity: None,
            },
        );
        print_skill_path("remote-skill", &skill, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_skill_path_remote_no_lockfile() {
        let skill = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        print_skill_path("remote-skill", &skill, None).unwrap();
    }

    #[test]
    fn print_agent_path_local() {
        let agent = AgentDependency {
            git: None,
            path: "./agents/my-agent.md".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: Some("opus".to_string()),
            parameters: Default::default(),
        };
        print_agent_path("my-agent", &agent, None).unwrap();
    }

    #[test]
    fn print_agent_path_remote_with_lockfile() {
        let agent = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/helper.md".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        let mut lockfile = Lockfile::new();
        lockfile.agents.insert(
            "remote-agent".to_string(),
            LockedAgent {
                source: LockedSource::Git {
                    git: "https://github.com/someone/agents.git".to_string(),
                    path: None,
                },
                git_commit: Some("fedcba0987654321".to_string()),
                integrity: None,
            },
        );
        print_agent_path("remote-agent", &agent, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_agent_path_remote_no_lockfile() {
        let agent = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/helper.md".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        print_agent_path("remote-agent", &agent, None).unwrap();
    }

    // ---- Tests for the run() function ----

    #[test]
    fn run_with_plugin_in_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
plugins:
  my-plugin:
    git: https://github.com/someone/plugin.git
    tag: v1.0.0
"#,
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "my-plugin".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_local_skill_in_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
skills:
  my-skill:
    path: ./skills/my-skill
"#,
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "my-skill".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_local_agent_in_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
agents:
  my-agent:
    path: ./agents/my-agent.md
    model: opus
"#,
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "my-agent".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_asset_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            "name: test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "nonexistent".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn run_no_manifest() {
        let dir = tempfile::tempdir().unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "something".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
    }

    #[test]
    fn run_with_lockfile_plugin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
plugins:
  locked-plugin:
    git: https://github.com/someone/plugin.git
    tag: v1.0.0
"#,
        )
        .unwrap();

        // Create lockfile
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "locked-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("abc123def456".to_string()),
                integrity: None,
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "locked-plugin".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_lockfile_remote_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
skills:
  remote-skill:
    git: https://github.com/someone/skills.git
    path: skills/remote
    tag: v1.0.0
"#,
        )
        .unwrap();

        let mut lockfile = Lockfile::new();
        lockfile.skills.insert(
            "remote-skill".to_string(),
            LockedSkill {
                source: LockedSource::Git {
                    git: "https://github.com/someone/skills.git".to_string(),
                    path: None,
                },
                git_commit: Some("aabbccdd".to_string()),
                integrity: None,
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "remote-skill".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_lockfile_remote_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
agents:
  remote-agent:
    git: https://github.com/someone/agents.git
    path: agents/helper.md
    tag: v2.0.0
"#,
        )
        .unwrap();

        let mut lockfile = Lockfile::new();
        lockfile.agents.insert(
            "remote-agent".to_string(),
            LockedAgent {
                source: LockedSource::Git {
                    git: "https://github.com/someone/agents.git".to_string(),
                    path: None,
                },
                git_commit: Some("fedcba09".to_string()),
                integrity: None,
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(WhichArgs {
            name: "remote-agent".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn print_skill_path_remote_lockfile_no_commit() {
        // Lockfile entry exists but has no git_commit — should fall through to fallback
        let skill = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        let mut lockfile = Lockfile::new();
        lockfile.skills.insert(
            "no-commit-skill".to_string(),
            LockedSkill {
                source: LockedSource::Git {
                    git: "https://github.com/someone/skills.git".to_string(),
                    path: None,
                },
                git_commit: None,
                integrity: None,
            },
        );
        print_skill_path("no-commit-skill", &skill, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_agent_path_remote_lockfile_no_commit() {
        let agent = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/helper.md".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        let mut lockfile = Lockfile::new();
        lockfile.agents.insert(
            "no-commit-agent".to_string(),
            LockedAgent {
                source: LockedSource::Git {
                    git: "https://github.com/someone/agents.git".to_string(),
                    path: None,
                },
                git_commit: None,
                integrity: None,
            },
        );
        print_agent_path("no-commit-agent", &agent, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_plugin_path_git_no_commit() {
        // Plugin with git source but no commit in lockfile entry
        let plugin = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "no-commit-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: None,
                integrity: None,
            },
        );
        print_plugin_path("no-commit-plugin", &plugin, Some(&lockfile)).unwrap();
    }
}
