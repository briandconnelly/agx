//! `agx info` — show details about an installed asset.

use anyhow::{bail, Context, Result};
use clap::Args;
use console::style;

use agx::cache;
use agx::config;
use agx::lockfile;
use agx::lockfile::types::{LockedSource, Lockfile};
use agx::manifest;
use agx::manifest::types::AgentEnvironment;
use agx::sources::registry;

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Asset name
    pub name: String,
}

/// The kind of asset found in the manifest.
enum AssetKind {
    Plugin,
    Skill,
    Agent,
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetKind::Plugin => write!(f, "plugin"),
            AssetKind::Skill => write!(f, "skill"),
            AssetKind::Agent => write!(f, "agent"),
        }
    }
}

pub fn run(args: InfoArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let project_dir = manifest_path.parent().expect("manifest must have a parent directory");
    let manifest = manifest::load(&manifest_path)?;

    // Try to load lockfile (optional — may not exist yet)
    let lockfile_path = project_dir.join(config::LOCKFILE_FILE);
    let lockfile = if lockfile_path.exists() {
        Some(lockfile::load(&lockfile_path)?)
    } else {
        None
    };

    // Find the asset in the manifest
    let name = &args.name;

    if let Some(plugin) = manifest.plugins.get(name) {
        print_header(name, &AssetKind::Plugin);
        print_plugin_info(name, plugin, &manifest, lockfile.as_ref())?;
    } else if let Some(skill) = manifest.skills.get(name) {
        print_header(name, &AssetKind::Skill);
        print_skill_info(name, skill, lockfile.as_ref())?;
    } else if let Some(agent) = manifest.agents.get(name) {
        print_header(name, &AssetKind::Agent);
        print_agent_info(name, agent, lockfile.as_ref())?;
    } else {
        bail!("asset '{}' not found in manifest", name);
    }

    Ok(())
}

/// Print the header line: "name (type)".
fn print_header(name: &str, kind: &AssetKind) {
    println!(
        "{} ({})",
        style(name).bold().cyan(),
        style(kind.to_string()).dim()
    );
}

/// Print a labeled info line with consistent indentation.
fn print_field(label: &str, value: &str) {
    println!("  {:<12}{}", style(format!("{label}:")).dim(), value);
}

/// Print info for a plugin dependency.
fn print_plugin_info(
    name: &str,
    plugin: &agx::manifest::types::PluginDependency,
    manifest: &AgentEnvironment,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    // Source
    if let Some(registry) = plugin.registry.as_deref() {
        print_field("Source", &format!("registry: {registry}"));
        if let Some(version) = plugin.version.as_deref() {
            // Check lockfile for resolved version
            if let Some(resolved) = lockfile.and_then(|lf| lf.plugins.get(name)) {
                if let LockedSource::Registry { resolved_version, .. } = &resolved.source {
                    print_field(
                        "Version",
                        &format!("{resolved_version} (constraint: {version})"),
                    );
                } else {
                    print_field("Version", &format!("constraint: {version}"));
                }

                // Commit
                if let Some(commit) = resolved.git_commit.as_deref() {
                    print_field("Commit", commit);
                }

                // Integrity
                if let Some(integrity) = resolved.integrity.as_deref() {
                    print_field("Integrity", integrity);
                }
            } else {
                print_field("Version", &format!("constraint: {version}"));
            }
        }

        // Check if registry URL is available
        if let Some(reg) = manifest.registries.get(registry) {
            print_field("Registry URL", &reg.url);
        }
    } else if let Some(git_url) = plugin.git.as_deref() {
        print_field("Source", &format!("git: {git_url}"));
        if let Some(tag) = plugin.tag.as_deref() {
            print_field("Tag", tag);
        }
        if let Some(branch) = plugin.branch.as_deref() {
            print_field("Branch", branch);
        }
        if let Some(commit) = plugin.commit.as_deref() {
            print_field("Commit", commit);
        }

        // Check lockfile for resolved info
        if let Some(resolved) = lockfile.and_then(|lf| lf.plugins.get(name)) {
            if let Some(commit) = resolved.git_commit.as_deref() {
                print_field("Locked commit", commit);
            }
            if let Some(integrity) = resolved.integrity.as_deref() {
                print_field("Integrity", integrity);
            }
        }
    }

    // Cache location
    if let Some(resolved) = lockfile.and_then(|lf| lf.plugins.get(name)) {
        let cache_key = match &resolved.source {
            LockedSource::Registry { resolved_version, .. } => Some(resolved_version.clone()),
            _ => resolved.git_commit.clone(),
        };
        if let Some(key) = &cache_key {
            if let Ok(Some(path)) = cache::get_plugin(name, key) {
                print_field("Cached at", &path.display().to_string());
            }
        }
    }

    // Registry metadata (fetch if available)
    if let Some(reg_name) = plugin.registry.as_deref() {
        if let Some(reg_config) = manifest.registries.get(reg_name) {
            let reg_cache = cache::registry_dir().ok();
            if let Some(ref reg_cache_dir) = reg_cache {
                let slug = agx::sources::git::url_to_slug(&reg_config.url);
                let reg_dir = reg_cache_dir.join(&slug);
                if reg_dir.exists() {
                    if let Ok(index) = registry::fetch_index_pinned(
                        &reg_config.url,
                        &reg_dir,
                        reg_config.pin.as_deref(),
                    ) {
                        if let Some(entry) = index.plugins.get(name) {
                            print_registry_metadata(entry);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Print enriched metadata from a registry plugin entry.
fn print_registry_metadata(entry: &registry::PluginEntry) {
    if let Some(ref author) = entry.author {
        print_field("Author", author);
    }
    if let Some(ref homepage) = entry.homepage {
        print_field("Homepage", homepage);
    }
    if let Some(ref repository) = entry.repository {
        print_field("Repository", repository);
    }
    if let Some(ref license) = entry.license {
        print_field("License", license);
    }
    if let Some(ref category) = entry.category {
        print_field("Category", category);
    }
    if !entry.keywords.is_empty() {
        print_field("Keywords", &entry.keywords.join(", "));
    }
    if !entry.tags.is_empty() {
        print_field("Tags", &entry.tags.join(", "));
    }
}

/// Print info for a skill dependency.
fn print_skill_info(
    name: &str,
    skill: &agx::manifest::types::SkillDependency,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    if skill.is_local() {
        print_field("Source", &format!("local: {}", skill.path));
    } else if let Some(git_url) = skill.git.as_deref() {
        print_field("Source", &format!("git: {git_url}"));
        print_field("Path", &skill.path);
        if let Some(tag) = skill.tag.as_deref() {
            print_field("Tag", tag);
        }
        if let Some(branch) = skill.branch.as_deref() {
            print_field("Branch", branch);
        }
        if let Some(commit) = skill.commit.as_deref() {
            print_field("Commit", commit);
        }
    }

    // Lockfile resolved info
    if let Some(resolved) = lockfile.and_then(|lf| lf.skills.get(name)) {
        if let Some(commit) = resolved.git_commit.as_deref() {
            print_field("Locked commit", commit);
        }
        if let Some(integrity) = resolved.integrity.as_deref() {
            print_field("Integrity", integrity);
        }

        // Cache location
        if let Some(commit) = resolved.git_commit.as_deref() {
            if let Ok(Some(path)) = cache::get_skill(name, commit) {
                print_field("Cached at", &path.display().to_string());
            }
        }
    }

    Ok(())
}

/// Print info for an agent dependency.
fn print_agent_info(
    name: &str,
    agent: &agx::manifest::types::AgentDependency,
    lockfile: Option<&Lockfile>,
) -> Result<()> {
    if agent.is_local() {
        print_field("Source", &format!("local: {}", agent.path));
    } else if let Some(git_url) = agent.git.as_deref() {
        print_field("Source", &format!("git: {git_url}"));
        print_field("Path", &agent.path);
        if let Some(tag) = agent.tag.as_deref() {
            print_field("Tag", tag);
        }
        if let Some(branch) = agent.branch.as_deref() {
            print_field("Branch", branch);
        }
        if let Some(commit) = agent.commit.as_deref() {
            print_field("Commit", commit);
        }
    }

    // Model override
    if let Some(model) = agent.model.as_deref() {
        print_field("Model", model);
    }

    // Lockfile resolved info
    if let Some(resolved) = lockfile.and_then(|lf| lf.agents.get(name)) {
        if let Some(commit) = resolved.git_commit.as_deref() {
            print_field("Locked commit", commit);
        }
        if let Some(integrity) = resolved.integrity.as_deref() {
            print_field("Integrity", integrity);
        }

        // Cache location
        if let Some(commit) = resolved.git_commit.as_deref() {
            if let Ok(Some(path)) = cache::get_agent(name, commit) {
                print_field("Cached at", &path.display().to_string());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::lockfile::types::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn test_manifest() -> AgentEnvironment {
        serde_yaml::from_str(
            r#"
name: test-project
backends:
  claude-code:
    isolation: strict
registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official
plugins:
  quality-review:
    registry: official
    version: ">=1.0.0, <2.0.0"
  custom-plugin:
    git: https://github.com/someone/their-plugin.git
    tag: v3.0.0
skills:
  deploy-checklist:
    path: ./skills/deploy-checklist
  remote-skill:
    git: https://github.com/someone/skills.git
    path: skills/remote
    tag: v1.0.0
agents:
  my-reviewer:
    path: ./agents/my-reviewer.md
    model: opus
  remote-agent:
    git: https://github.com/someone/agents.git
    path: agents/helper.md
    tag: v2.0.0
"#,
        )
        .unwrap()
    }

    fn test_lockfile() -> Lockfile {
        let mut lf = Lockfile::new();
        lf.plugins.insert(
            "quality-review".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    resolved_version: "1.2.0".to_string(),
                    path: None,
                },
                git_commit: Some("a1b2c3d4e5f6".to_string()),
                integrity: Some("sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b".to_string()),
            },
        );
        lf.skills.insert(
            "deploy-checklist".to_string(),
            LockedSkill {
                source: LockedSource::Local {
                    local: "./skills/deploy-checklist".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        lf.agents.insert(
            "my-reviewer".to_string(),
            LockedAgent {
                source: LockedSource::Local {
                    local: "./agents/my-reviewer.md".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        lf
    }

    #[test]
    fn find_plugin_in_manifest() {
        let manifest = test_manifest();
        assert!(manifest.plugins.contains_key("quality-review"));
        assert!(manifest.plugins.contains_key("custom-plugin"));
    }

    #[test]
    fn find_skill_in_manifest() {
        let manifest = test_manifest();
        assert!(manifest.skills.contains_key("deploy-checklist"));
        assert!(manifest.skills["deploy-checklist"].is_local());
    }

    #[test]
    fn find_agent_in_manifest() {
        let manifest = test_manifest();
        assert!(manifest.agents.contains_key("my-reviewer"));
        assert!(manifest.agents["my-reviewer"].is_local());
        assert_eq!(manifest.agents["my-reviewer"].model.as_deref(), Some("opus"));
    }

    #[test]
    fn lockfile_has_resolved_plugin() {
        let lf = test_lockfile();
        let plugin = &lf.plugins["quality-review"];
        match &plugin.source {
            LockedSource::Registry { resolved_version, .. } => {
                assert_eq!(resolved_version, "1.2.0");
            }
            _ => panic!("expected registry source"),
        }
        assert!(plugin.git_commit.is_some());
        assert!(plugin.integrity.is_some());
    }

    #[test]
    fn asset_not_found_errors() {
        let manifest = test_manifest();
        assert!(!manifest.plugins.contains_key("nonexistent"));
        assert!(!manifest.skills.contains_key("nonexistent"));
        assert!(!manifest.agents.contains_key("nonexistent"));
    }

    #[test]
    fn git_plugin_info() {
        let manifest = test_manifest();
        let plugin = &manifest.plugins["custom-plugin"];
        assert!(plugin.is_git());
        assert_eq!(plugin.git.as_deref(), Some("https://github.com/someone/their-plugin.git"));
        assert_eq!(plugin.tag.as_deref(), Some("v3.0.0"));
    }

    #[test]
    fn remote_skill_info() {
        let manifest = test_manifest();
        let skill = &manifest.skills["remote-skill"];
        assert!(!skill.is_local());
        assert_eq!(skill.git.as_deref(), Some("https://github.com/someone/skills.git"));
        assert_eq!(skill.path, "skills/remote");
    }

    #[test]
    fn remote_agent_info() {
        let manifest = test_manifest();
        let agent = &manifest.agents["remote-agent"];
        assert!(!agent.is_local());
        assert_eq!(agent.git.as_deref(), Some("https://github.com/someone/agents.git"));
        assert_eq!(agent.path, "agents/helper.md");
    }

    // ---- Tests that exercise the print_* functions for line coverage ----

    #[test]
    fn print_header_plugin() {
        print_header("my-plugin", &AssetKind::Plugin);
    }

    #[test]
    fn print_header_skill() {
        print_header("my-skill", &AssetKind::Skill);
    }

    #[test]
    fn print_header_agent() {
        print_header("my-agent", &AssetKind::Agent);
    }

    #[test]
    fn print_field_sample() {
        print_field("Source", "git: https://example.com/repo.git");
        print_field("Version", "1.2.3");
    }

    #[test]
    fn print_plugin_info_registry_with_lockfile() {
        let manifest = test_manifest();
        let lockfile = test_lockfile();
        let plugin = &manifest.plugins["quality-review"];
        print_plugin_info("quality-review", plugin, &manifest, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_plugin_info_registry_without_lockfile() {
        let manifest = test_manifest();
        let plugin = &manifest.plugins["quality-review"];
        print_plugin_info("quality-review", plugin, &manifest, None).unwrap();
    }

    #[test]
    fn print_plugin_info_registry_with_lockfile_no_match() {
        // Plugin is in manifest but not in lockfile
        let manifest = test_manifest();
        let lockfile = Lockfile::new(); // empty lockfile
        let plugin = &manifest.plugins["quality-review"];
        print_plugin_info("quality-review", plugin, &manifest, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_plugin_info_git_with_tag() {
        let manifest = test_manifest();
        let plugin = &manifest.plugins["custom-plugin"];
        print_plugin_info("custom-plugin", plugin, &manifest, None).unwrap();
    }

    #[test]
    fn print_plugin_info_git_with_branch() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
plugins:
  branched-plugin:
    git: https://github.com/someone/plugin.git
    branch: main
"#,
        )
        .unwrap();
        let plugin = &manifest.plugins["branched-plugin"];
        print_plugin_info("branched-plugin", plugin, &manifest, None).unwrap();
    }

    #[test]
    fn print_plugin_info_git_with_commit() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
plugins:
  commit-plugin:
    git: https://github.com/someone/plugin.git
    commit: abc123def456
"#,
        )
        .unwrap();
        let plugin = &manifest.plugins["commit-plugin"];
        print_plugin_info("commit-plugin", plugin, &manifest, None).unwrap();
    }

    #[test]
    fn print_plugin_info_git_with_lockfile() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
plugins:
  git-plugin:
    git: https://github.com/someone/plugin.git
    tag: v1.0.0
"#,
        )
        .unwrap();
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "git-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("deadbeef12345678".to_string()),
                integrity: Some("sha256:abcdef1234567890".to_string()),
            },
        );
        let plugin = &manifest.plugins["git-plugin"];
        print_plugin_info("git-plugin", plugin, &manifest, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_skill_info_local() {
        let manifest = test_manifest();
        let skill = &manifest.skills["deploy-checklist"];
        print_skill_info("deploy-checklist", skill, None).unwrap();
    }

    #[test]
    fn print_skill_info_remote_with_tag() {
        let manifest = test_manifest();
        let skill = &manifest.skills["remote-skill"];
        print_skill_info("remote-skill", skill, None).unwrap();
    }

    #[test]
    fn print_skill_info_remote_with_lockfile() {
        let manifest = test_manifest();
        let skill = &manifest.skills["remote-skill"];
        let mut lockfile = Lockfile::new();
        lockfile.skills.insert(
            "remote-skill".to_string(),
            LockedSkill {
                source: LockedSource::Git {
                    git: "https://github.com/someone/skills.git".to_string(),
                    path: None,
                },
                git_commit: Some("aabbccdd11223344".to_string()),
                integrity: Some("sha256:1234abcd5678efgh".to_string()),
            },
        );
        print_skill_info("remote-skill", skill, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_skill_info_remote_with_branch() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
skills:
  branched-skill:
    git: https://github.com/someone/skills.git
    path: skills/x
    branch: develop
"#,
        )
        .unwrap();
        let skill = &manifest.skills["branched-skill"];
        print_skill_info("branched-skill", skill, None).unwrap();
    }

    #[test]
    fn print_skill_info_remote_with_commit() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
skills:
  commit-skill:
    git: https://github.com/someone/skills.git
    path: skills/x
    commit: abcdef1234567890
"#,
        )
        .unwrap();
        let skill = &manifest.skills["commit-skill"];
        print_skill_info("commit-skill", skill, None).unwrap();
    }

    #[test]
    fn print_agent_info_local_with_model() {
        let manifest = test_manifest();
        let agent = &manifest.agents["my-reviewer"];
        print_agent_info("my-reviewer", agent, None).unwrap();
    }

    #[test]
    fn print_agent_info_remote_with_tag() {
        let manifest = test_manifest();
        let agent = &manifest.agents["remote-agent"];
        print_agent_info("remote-agent", agent, None).unwrap();
    }

    #[test]
    fn print_agent_info_remote_with_lockfile() {
        let manifest = test_manifest();
        let agent = &manifest.agents["remote-agent"];
        let mut lockfile = Lockfile::new();
        lockfile.agents.insert(
            "remote-agent".to_string(),
            LockedAgent {
                source: LockedSource::Git {
                    git: "https://github.com/someone/agents.git".to_string(),
                    path: None,
                },
                git_commit: Some("fedcba0987654321".to_string()),
                integrity: Some("sha256:9876fedc5432abcd".to_string()),
            },
        );
        print_agent_info("remote-agent", agent, Some(&lockfile)).unwrap();
    }

    #[test]
    fn print_agent_info_remote_with_branch() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
agents:
  branched-agent:
    git: https://github.com/someone/agents.git
    path: agents/x.md
    branch: staging
"#,
        )
        .unwrap();
        let agent = &manifest.agents["branched-agent"];
        print_agent_info("branched-agent", agent, None).unwrap();
    }

    #[test]
    fn print_agent_info_remote_with_commit() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code:
    isolation: strict
agents:
  commit-agent:
    git: https://github.com/someone/agents.git
    path: agents/x.md
    commit: 1122334455667788
"#,
        )
        .unwrap();
        let agent = &manifest.agents["commit-agent"];
        print_agent_info("commit-agent", agent, None).unwrap();
    }

    #[test]
    fn print_registry_metadata_full() {
        let entry = registry::PluginEntry {
            description: "A great plugin".to_string(),
            versions: BTreeMap::new(),
            author: Some("Test Author".to_string()),
            homepage: Some("https://example.com".to_string()),
            repository: Some("https://github.com/test/plugin".to_string()),
            license: Some("MIT".to_string()),
            keywords: vec!["testing".to_string(), "awesome".to_string()],
            tags: vec!["stable".to_string(), "v1".to_string()],
            category: Some("Utilities".to_string()),
            strict: None,
        };
        print_registry_metadata(&entry);
    }

    #[test]
    fn print_registry_metadata_minimal() {
        let entry = registry::PluginEntry {
            description: "Minimal plugin".to_string(),
            versions: BTreeMap::new(),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: vec![],
            tags: vec![],
            category: None,
            strict: None,
        };
        print_registry_metadata(&entry);
    }

    #[test]
    fn asset_kind_display() {
        assert_eq!(AssetKind::Plugin.to_string(), "plugin");
        assert_eq!(AssetKind::Skill.to_string(), "skill");
        assert_eq!(AssetKind::Agent.to_string(), "agent");
    }

    // ---- run() function tests ----

    #[test]
    fn run_with_plugin() {
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
        let result = run(InfoArgs {
            name: "my-plugin".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_skill() {
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
        let result = run(InfoArgs {
            name: "my-skill".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_agent() {
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
        let result = run(InfoArgs {
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
        let result = run(InfoArgs {
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
        let result = run(InfoArgs {
            name: "something".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
    }

    #[test]
    fn run_with_lockfile_registry_plugin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
registries:
  official:
    url: https://github.com/example/registry
plugins:
  my-plugin:
    registry: official
    version: ">=1.0.0"
"#,
        )
        .unwrap();

        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "my-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    resolved_version: "1.5.0".to_string(),
                    path: None,
                },
                git_commit: Some("deadbeef".to_string()),
                integrity: Some("sha256:abc123".to_string()),
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(InfoArgs {
            name: "my-plugin".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_with_lockfile_git_plugin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent-environment.yml"),
            r#"name: test
backends:
  claude-code: {}
plugins:
  git-plugin:
    git: https://github.com/someone/plugin.git
    tag: v1.0.0
"#,
        )
        .unwrap();

        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "git-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("deadbeef12345678".to_string()),
                integrity: Some("sha256:abcdef1234567890".to_string()),
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(InfoArgs {
            name: "git-plugin".to_string(),
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
                integrity: Some("sha256:1234".to_string()),
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(InfoArgs {
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
                integrity: Some("sha256:9876".to_string()),
            },
        );
        agx::lockfile::save(&lockfile, &dir.path().join("agent-environment.lock")).unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(InfoArgs {
            name: "remote-agent".to_string(),
        });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn print_plugin_info_registry_non_git_lockfile_source() {
        // Test the else branch where lockfile has a registry source but plugin asks for version
        // and the locked source is not Registry variant
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code: {}
registries:
  official:
    url: https://github.com/example/registry
plugins:
  my-plugin:
    registry: official
    version: ">=1.0.0"
"#,
        )
        .unwrap();
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "my-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("abc123".to_string()),
                integrity: None,
            },
        );
        let plugin = &manifest.plugins["my-plugin"];
        print_plugin_info("my-plugin", plugin, &manifest, Some(&lockfile)).unwrap();
    }
}
