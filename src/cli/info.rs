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
    let project_dir = manifest_path.parent().unwrap();
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
}
