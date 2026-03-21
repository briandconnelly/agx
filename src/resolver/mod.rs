//! Version resolution and conflict detection.
//!
//! The resolver takes a parsed manifest and pre-fetched registry/git data,
//! then produces a complete lockfile with pinned versions. It performs no I/O.

pub mod semver_utils;

use crate::lockfile::types::{LockedAgent, LockedPlugin, LockedSkill, LockedSource, Lockfile};
use crate::manifest::types::AgentEnvironment;
use semver_utils::ResolverError;
use std::collections::BTreeMap;

/// Available version information for a single plugin version in a registry.
#[derive(Debug, Clone)]
pub struct PluginVersionInfo {
    /// The semver version.
    pub version: semver::Version,
    /// The git commit SHA corresponding to this version.
    pub git_commit: String,
    /// Path within the registry repo (if any).
    pub path: Option<String>,
    /// How to fetch this plugin (None for legacy/registry-local entries).
    pub source_kind: Option<crate::sources::registry::PluginSourceKind>,
}

/// Pre-fetched data needed for resolution.
///
/// In production, this is populated by the registry and git clients.
/// In tests, it can be constructed directly with mock data.
pub struct ResolveContext {
    /// Registry name -> plugin name -> available versions.
    pub registry_versions: BTreeMap<String, BTreeMap<String, Vec<PluginVersionInfo>>>,
    /// Git URL -> resolved commit SHA (pre-resolved by the git client).
    pub git_commits: BTreeMap<String, String>,
}

/// Names of assets provided by a plugin, used for conflict detection.
#[derive(Debug, Clone, Default)]
pub struct PluginAssets {
    /// Agent names this plugin provides.
    pub agents: Vec<String>,
    /// Skill names this plugin provides.
    pub skills: Vec<String>,
}

/// Resolve all dependencies in a manifest, producing a complete lockfile.
///
/// This processes all plugins, skills, and agents in the manifest. For registry
/// plugins it selects the best matching version; for git sources it looks up the
/// pre-resolved commit SHA; for local assets it records the path directly.
///
/// The `plugin_assets` map describes which agents/skills each plugin provides,
/// used for conflict detection.
pub fn resolve_all(
    manifest: &AgentEnvironment,
    context: &ResolveContext,
    plugin_assets: &BTreeMap<String, PluginAssets>,
) -> Result<Lockfile, ResolverError> {
    let mut lockfile = Lockfile::new();

    // Resolve plugins
    for (name, dep) in &manifest.plugins {
        let locked = resolve_plugin(name, dep, context)?;
        lockfile.plugins.insert(name.clone(), locked);
    }

    // Resolve skills
    for (name, dep) in &manifest.skills {
        let locked = resolve_skill(name, dep, context)?;
        lockfile.skills.insert(name.clone(), locked);
    }

    // Resolve agents
    for (name, dep) in &manifest.agents {
        let locked = resolve_agent(name, dep, context)?;
        lockfile.agents.insert(name.clone(), locked);
    }

    // Conflict detection: check if multiple plugins provide the same asset name
    detect_conflicts(manifest, plugin_assets)?;

    Ok(lockfile)
}

/// Resolve a single dependency by name, updating an existing lockfile.
///
/// Looks up the named entry in the manifest's plugins, skills, or agents,
/// resolves it, and returns a new lockfile with that entry updated.
pub fn resolve_single(
    name: &str,
    manifest: &AgentEnvironment,
    existing_lockfile: &Lockfile,
    context: &ResolveContext,
) -> Result<Lockfile, ResolverError> {
    let mut lockfile = existing_lockfile.clone();

    if let Some(dep) = manifest.plugins.get(name) {
        let locked = resolve_plugin(name, dep, context)?;
        lockfile.plugins.insert(name.to_string(), locked);
        return Ok(lockfile);
    }

    if let Some(dep) = manifest.skills.get(name) {
        let locked = resolve_skill(name, dep, context)?;
        lockfile.skills.insert(name.to_string(), locked);
        return Ok(lockfile);
    }

    if let Some(dep) = manifest.agents.get(name) {
        let locked = resolve_agent(name, dep, context)?;
        lockfile.agents.insert(name.to_string(), locked);
        return Ok(lockfile);
    }

    Err(ResolverError::NotFound {
        name: name.to_string(),
    })
}

/// Resolve a single plugin dependency to a locked entry.
fn resolve_plugin(
    name: &str,
    dep: &crate::manifest::types::PluginDependency,
    context: &ResolveContext,
) -> Result<LockedPlugin, ResolverError> {
    if dep.is_registry() {
        let registry_name = dep.registry.as_deref().unwrap();
        let constraint_str = dep.version.as_deref().unwrap_or("*");
        let constraint = semver_utils::parse_constraint(constraint_str)?;

        let registry = context
            .registry_versions
            .get(registry_name)
            .ok_or_else(|| ResolverError::RegistryNotFound {
                registry: registry_name.to_string(),
            })?;

        let available = registry
            .get(name)
            .ok_or_else(|| ResolverError::NoMatch {
                package: name.to_string(),
                constraint: constraint_str.to_string(),
            })?;

        let versions: Vec<semver::Version> =
            available.iter().map(|v| v.version.clone()).collect();
        let best = semver_utils::resolve_best(&versions, &constraint).ok_or_else(|| {
            ResolverError::NoMatch {
                package: name.to_string(),
                constraint: constraint_str.to_string(),
            }
        })?;

        // Find the corresponding PluginVersionInfo to get the commit SHA
        let info = available
            .iter()
            .find(|v| v.version == best)
            .expect("version was in the list");

        // Determine the locked source based on the plugin's source kind.
        // Registry-local plugins use LockedSource::Registry; external git
        // sources are expanded to LockedSource::Git; npm uses LockedSource::Npm.
        use crate::sources::registry::PluginSourceKind;
        let (source, git_commit) = match &info.source_kind {
            Some(PluginSourceKind::GitHub { repo, git_ref, sha }) => {
                let url = format!("https://github.com/{}.git", repo);
                let commit = if let Some(s) = sha {
                    s.clone()
                } else {
                    resolve_git_ref_from_context(&url, git_ref.as_deref(), context)?
                };
                (
                    LockedSource::Git {
                        git: url,
                        path: None,
                    },
                    Some(commit),
                )
            }
            Some(PluginSourceKind::GitUrl { url, git_ref, sha }) => {
                let commit = if let Some(s) = sha {
                    s.clone()
                } else {
                    resolve_git_ref_from_context(url, git_ref.as_deref(), context)?
                };
                (
                    LockedSource::Git {
                        git: url.clone(),
                        path: None,
                    },
                    Some(commit),
                )
            }
            Some(PluginSourceKind::GitSubdir { url, path, git_ref, sha }) => {
                let expanded = expand_source_url(url);
                let commit = if let Some(s) = sha {
                    s.clone()
                } else {
                    resolve_git_ref_from_context(&expanded, git_ref.as_deref(), context)?
                };
                (
                    LockedSource::Git {
                        git: expanded,
                        path: Some(path.clone()),
                    },
                    Some(commit),
                )
            }
            Some(PluginSourceKind::Npm { package, version, registry }) => {
                (
                    LockedSource::Npm {
                        package: package.clone(),
                        resolved_version: version.clone(),
                        npm_registry: registry.clone(),
                    },
                    None,
                )
            }
            Some(PluginSourceKind::RegistryLocal { .. }) | None => {
                // Standard registry-local plugin
                (
                    LockedSource::Registry {
                        registry: registry_name.to_string(),
                        resolved_version: best.to_string(),
                        path: info.path.clone(),
                    },
                    Some(info.git_commit.clone()),
                )
            }
        };

        Ok(LockedPlugin {
            source,
            git_commit,
            integrity: None,
        })
    } else if dep.is_git() {
        let git_url = dep.git.as_deref().unwrap();
        let commit = resolve_git_ref(git_url, dep.tag.as_deref(), dep.branch.as_deref(), dep.commit.as_deref(), context)?;
        Ok(LockedPlugin {
            source: LockedSource::Git {
                git: git_url.to_string(),
                path: None,
            },
            git_commit: Some(commit),
            integrity: None,
        })
    } else {
        Err(ResolverError::InvalidSource {
            name: name.to_string(),
            reason: "plugin must specify either registry or git source".to_string(),
        })
    }
}

/// Shared helper for resolving a skill or agent dependency.
///
/// Both skill and agent dependencies follow the same pattern: they are either
/// local (path-only) or git-sourced (git URL + path + ref). This function
/// handles both cases and returns a `LockedEntry`.
fn resolve_path_or_git_dep(
    name: &str,
    kind: &str,
    is_local: bool,
    path: &str,
    git: Option<&str>,
    tag: Option<&str>,
    branch: Option<&str>,
    commit: Option<&str>,
    context: &ResolveContext,
) -> Result<crate::lockfile::types::LockedEntry, ResolverError> {
    if is_local {
        Ok(crate::lockfile::types::LockedEntry {
            source: LockedSource::Local {
                local: path.to_string(),
            },
            git_commit: None,
            integrity: None,
        })
    } else if let Some(git_url) = git {
        let resolved_commit = resolve_git_ref(git_url, tag, branch, commit, context)?;
        Ok(crate::lockfile::types::LockedEntry {
            source: LockedSource::Git {
                git: git_url.to_string(),
                path: Some(path.to_string()),
            },
            git_commit: Some(resolved_commit),
            integrity: None,
        })
    } else {
        Err(ResolverError::InvalidSource {
            name: name.to_string(),
            reason: format!("{kind} must specify either a local path or git source"),
        })
    }
}

/// Resolve a single skill dependency to a locked entry.
fn resolve_skill(
    name: &str,
    dep: &crate::manifest::types::SkillDependency,
    context: &ResolveContext,
) -> Result<LockedSkill, ResolverError> {
    resolve_path_or_git_dep(
        name,
        "skill",
        dep.is_local(),
        &dep.path,
        dep.git.as_deref(),
        dep.tag.as_deref(),
        dep.branch.as_deref(),
        dep.commit.as_deref(),
        context,
    )
}

/// Resolve a single agent dependency to a locked entry.
fn resolve_agent(
    name: &str,
    dep: &crate::manifest::types::AgentDependency,
    context: &ResolveContext,
) -> Result<LockedAgent, ResolverError> {
    resolve_path_or_git_dep(
        name,
        "agent",
        dep.is_local(),
        &dep.path,
        dep.git.as_deref(),
        dep.tag.as_deref(),
        dep.branch.as_deref(),
        dep.commit.as_deref(),
        context,
    )
}

/// Resolve a git ref (tag, branch, or commit) to a commit SHA using the context.
///
/// If an explicit commit is provided, it is used directly. Otherwise, the git URL
/// (optionally with ref suffix) is looked up in the pre-resolved git_commits map.
fn resolve_git_ref(
    git_url: &str,
    tag: Option<&str>,
    branch: Option<&str>,
    commit: Option<&str>,
    context: &ResolveContext,
) -> Result<String, ResolverError> {
    // If a commit SHA is pinned directly, use it
    if let Some(sha) = commit {
        return Ok(sha.to_string());
    }

    // Build a lookup key: git_url + ref indicator
    let ref_str = tag.or(branch).unwrap_or("HEAD");
    let lookup_key = format!("{}#{}", git_url, ref_str);

    // Try the compound key first, then the bare URL
    context
        .git_commits
        .get(&lookup_key)
        .or_else(|| context.git_commits.get(git_url))
        .cloned()
        .ok_or_else(|| ResolverError::GitRefNotResolved {
            url: git_url.to_string(),
            git_ref: ref_str.to_string(),
        })
}

/// Look up a git commit from the resolve context for a URL + optional ref.
///
/// Used by registry plugins with external git sources (GitHub, GitUrl, GitSubdir).
fn resolve_git_ref_from_context(
    url: &str,
    git_ref: Option<&str>,
    context: &ResolveContext,
) -> Result<String, ResolverError> {
    let ref_str = git_ref.unwrap_or("HEAD");
    let lookup_key = format!("{url}#{ref_str}");

    context
        .git_commits
        .get(&lookup_key)
        .or_else(|| context.git_commits.get(url))
        .cloned()
        .ok_or_else(|| ResolverError::GitRefNotResolved {
            url: url.to_string(),
            git_ref: ref_str.to_string(),
        })
}

/// Expand a git URL that might be a GitHub `owner/repo` shorthand.
pub fn expand_source_url(url: &str) -> String {
    if !url.contains("://") && !url.contains('@') && url.matches('/').count() == 1 {
        format!("https://github.com/{}.git", url)
    } else {
        url.to_string()
    }
}

/// Detect asset name conflicts across plugins.
///
/// Two plugins providing the same-named agent or skill is an error, unless
/// include/exclude filters on the plugins resolve the conflict by ensuring
/// only one plugin actually provides that asset.
fn detect_conflicts(
    manifest: &AgentEnvironment,
    plugin_assets: &BTreeMap<String, PluginAssets>,
) -> Result<(), ResolverError> {
    // Track which plugin provides each agent/skill name
    let mut agent_providers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut skill_providers: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (plugin_name, dep) in &manifest.plugins {
        let assets = match plugin_assets.get(plugin_name) {
            Some(a) => a,
            None => continue, // no asset info available
        };

        for agent_name in &assets.agents {
            // Check if this agent is filtered out by include/exclude
            if is_asset_included(agent_name, &dep.include, &dep.exclude, AssetKind::Agent) {
                agent_providers
                    .entry(agent_name.clone())
                    .or_default()
                    .push(plugin_name.clone());
            }
        }

        for skill_name in &assets.skills {
            if is_asset_included(skill_name, &dep.include, &dep.exclude, AssetKind::Skill) {
                skill_providers
                    .entry(skill_name.clone())
                    .or_default()
                    .push(plugin_name.clone());
            }
        }
    }

    // Check for conflicts
    for (asset_name, providers) in &agent_providers {
        if providers.len() > 1 {
            return Err(ResolverError::Conflict {
                name: asset_name.clone(),
                sources: providers.clone(),
            });
        }
    }

    for (asset_name, providers) in &skill_providers {
        if providers.len() > 1 {
            return Err(ResolverError::Conflict {
                name: asset_name.clone(),
                sources: providers.clone(),
            });
        }
    }

    Ok(())
}

/// Asset kind for include/exclude filtering.
enum AssetKind {
    Agent,
    Skill,
}

/// Check if a named asset is included based on include/exclude filters.
fn is_asset_included(
    name: &str,
    include: &Option<crate::manifest::types::AssetFilter>,
    exclude: &Option<crate::manifest::types::AssetFilter>,
    kind: AssetKind,
) -> bool {
    // If there's an include filter, the asset must be in it
    if let Some(filter) = include {
        let list = match kind {
            AssetKind::Agent => &filter.agents,
            AssetKind::Skill => &filter.skills,
        };
        if !list.is_empty() && !list.iter().any(|n| n == name) {
            return false;
        }
    }

    // If there's an exclude filter, the asset must not be in it
    if let Some(filter) = exclude {
        let list = match kind {
            AssetKind::Agent => &filter.agents,
            AssetKind::Skill => &filter.skills,
        };
        if list.iter().any(|n| n == name) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::types::LockedEntry;
    use crate::manifest::types::*;

    /// Helper: build a minimal manifest for testing.
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
  git-skill:
    git: https://github.com/someone/skills.git
    path: skills/deploy
    tag: v1.0.0

  local-skill:
    path: ./skills/local-skill

agents:
  my-reviewer:
    path: ./agents/my-reviewer.md
    model: opus
"#,
        )
        .unwrap()
    }

    /// Helper: build a ResolveContext with mock data.
    fn test_context() -> ResolveContext {
        let mut registry_versions = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "quality-review".to_string(),
            vec![
                PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "aaa111".to_string(),
                    path: None,
                    source_kind: None,
                },
                PluginVersionInfo {
                    version: semver::Version::new(1, 2, 0),
                    git_commit: "bbb222".to_string(),
                    path: None,
                    source_kind: None,
                },
                PluginVersionInfo {
                    version: semver::Version::new(1, 5, 0),
                    git_commit: "ccc333".to_string(),
                    path: None,
                    source_kind: None,
                },
                PluginVersionInfo {
                    version: semver::Version::new(2, 0, 0),
                    git_commit: "ddd444".to_string(),
                    path: None,
                    source_kind: None,
                },
            ],
        );
        registry_versions.insert("official".to_string(), official);

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/someone/their-plugin.git#v3.0.0".to_string(),
            "eee555".to_string(),
        );
        git_commits.insert(
            "https://github.com/someone/skills.git#v1.0.0".to_string(),
            "fff666".to_string(),
        );

        ResolveContext {
            registry_versions,
            git_commits,
        }
    }

    #[test]
    fn resolve_all_mixed_manifest() {
        let manifest = test_manifest();
        let context = test_context();
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();

        // Registry plugin: should pick 1.5.0 (best match for >=1.0.0, <2.0.0)
        let qr = &lockfile.plugins["quality-review"];
        match &qr.source {
            LockedSource::Registry {
                registry,
                resolved_version,
                ..
            } => {
                assert_eq!(registry, "official");
                assert_eq!(resolved_version, "1.5.0");
            }
            other => panic!("expected Registry source, got {:?}", other),
        }
        assert_eq!(qr.git_commit.as_deref(), Some("ccc333"));

        // Git plugin: should have resolved commit
        let cp = &lockfile.plugins["custom-plugin"];
        match &cp.source {
            LockedSource::Git { git, .. } => {
                assert_eq!(git, "https://github.com/someone/their-plugin.git");
            }
            other => panic!("expected Git source, got {:?}", other),
        }
        assert_eq!(cp.git_commit.as_deref(), Some("eee555"));

        // Git skill
        let gs = &lockfile.skills["git-skill"];
        match &gs.source {
            LockedSource::Git { git, path } => {
                assert_eq!(git, "https://github.com/someone/skills.git");
                assert_eq!(path.as_deref(), Some("skills/deploy"));
            }
            other => panic!("expected Git source, got {:?}", other),
        }
        assert_eq!(gs.git_commit.as_deref(), Some("fff666"));

        // Local skill: just a path, no commit
        let ls = &lockfile.skills["local-skill"];
        match &ls.source {
            LockedSource::Local { local } => {
                assert_eq!(local, "./skills/local-skill");
            }
            other => panic!("expected Local source, got {:?}", other),
        }
        assert!(ls.git_commit.is_none());

        // Local agent
        let agent = &lockfile.agents["my-reviewer"];
        match &agent.source {
            LockedSource::Local { local } => {
                assert_eq!(local, "./agents/my-reviewer.md");
            }
            other => panic!("expected Local source, got {:?}", other),
        }
    }

    #[test]
    fn conflict_detection_two_plugins_same_agent() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: conflict-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com

plugins:
  plugin-a:
    registry: official
    version: "^1.0.0"
  plugin-b:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let context = {
            let mut rv = BTreeMap::new();
            let mut official = BTreeMap::new();
            official.insert(
                "plugin-a".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "a1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            official.insert(
                "plugin-b".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "b1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            rv.insert("official".to_string(), official);
            ResolveContext {
                registry_versions: rv,
                git_commits: BTreeMap::new(),
            }
        };

        // Both plugins provide an agent named "shared-agent"
        let mut plugin_assets = BTreeMap::new();
        plugin_assets.insert(
            "plugin-a".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string()],
                skills: vec![],
            },
        );
        plugin_assets.insert(
            "plugin-b".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string()],
                skills: vec![],
            },
        );

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::Conflict { name, sources } => {
                assert_eq!(name, "shared-agent");
                assert_eq!(sources.len(), 2);
            }
            other => panic!("expected Conflict error, got {:?}", other),
        }
    }

    #[test]
    fn conflict_resolved_by_exclude_filter() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: filter-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com

plugins:
  plugin-a:
    registry: official
    version: "^1.0.0"
  plugin-b:
    registry: official
    version: "^1.0.0"
    exclude:
      agents:
        - shared-agent
"#,
        )
        .unwrap();

        let context = {
            let mut rv = BTreeMap::new();
            let mut official = BTreeMap::new();
            official.insert(
                "plugin-a".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "a1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            official.insert(
                "plugin-b".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "b1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            rv.insert("official".to_string(), official);
            ResolveContext {
                registry_versions: rv,
                git_commits: BTreeMap::new(),
            }
        };

        let mut plugin_assets = BTreeMap::new();
        plugin_assets.insert(
            "plugin-a".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string()],
                skills: vec![],
            },
        );
        plugin_assets.insert(
            "plugin-b".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string()],
                skills: vec![],
            },
        );

        // Should succeed because plugin-b excludes "shared-agent"
        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        assert_eq!(lockfile.plugins.len(), 2);
    }

    #[test]
    fn conflict_resolved_by_include_filter() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: include-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com

plugins:
  plugin-a:
    registry: official
    version: "^1.0.0"
    include:
      agents:
        - shared-agent
  plugin-b:
    registry: official
    version: "^1.0.0"
    include:
      agents:
        - other-agent
"#,
        )
        .unwrap();

        let context = {
            let mut rv = BTreeMap::new();
            let mut official = BTreeMap::new();
            official.insert(
                "plugin-a".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "a1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            official.insert(
                "plugin-b".to_string(),
                vec![PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "b1".to_string(),
                    path: None,
                    source_kind: None,
                }],
            );
            rv.insert("official".to_string(), official);
            ResolveContext {
                registry_versions: rv,
                git_commits: BTreeMap::new(),
            }
        };

        let mut plugin_assets = BTreeMap::new();
        plugin_assets.insert(
            "plugin-a".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string(), "other-agent".to_string()],
                skills: vec![],
            },
        );
        plugin_assets.insert(
            "plugin-b".to_string(),
            PluginAssets {
                agents: vec!["shared-agent".to_string(), "other-agent".to_string()],
                skills: vec![],
            },
        );

        // plugin-a only includes shared-agent, plugin-b only includes other-agent
        // No conflict.
        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        assert_eq!(lockfile.plugins.len(), 2);
    }

    #[test]
    fn resolve_single_updates_one_entry() {
        let manifest = test_manifest();
        let context = test_context();

        // Start with an empty lockfile
        let existing = Lockfile::new();

        let updated =
            resolve_single("quality-review", &manifest, &existing, &context).unwrap();

        assert_eq!(updated.plugins.len(), 1);
        assert!(updated.plugins.contains_key("quality-review"));
        // Skills and agents should still be empty
        assert!(updated.skills.is_empty());
        assert!(updated.agents.is_empty());
    }

    #[test]
    fn resolve_single_not_found() {
        let manifest = test_manifest();
        let context = test_context();
        let existing = Lockfile::new();

        let err = resolve_single("nonexistent", &manifest, &existing, &context).unwrap_err();
        match err {
            ResolverError::NotFound { name } => assert_eq!(name, "nonexistent"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn resolve_plugin_with_pinned_commit() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: commit-test
backends:
  claude-code: {}

plugins:
  pinned-plugin:
    git: https://github.com/someone/plugin.git
    commit: abc123def456
"#,
        )
        .unwrap();

        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let pp = &lockfile.plugins["pinned-plugin"];
        assert_eq!(pp.git_commit.as_deref(), Some("abc123def456"));
    }

    #[test]
    fn resolve_no_matching_version_error() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: nomatch-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  missing-plugin:
    registry: official
    version: "^5.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "missing-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "x".to_string(),
                path: None,
                source_kind: None,
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::NoMatch { package, .. } => assert_eq!(package, "missing-plugin"),
            other => panic!("expected NoMatch, got {:?}", other),
        }
    }

    // --- resolve_plugin with GitHub source_kind ---

    #[test]
    fn resolve_plugin_github_source_with_sha() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: gh-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  gh-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "gh-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitHub {
                    repo: "owner/repo".to_string(),
                    git_ref: Some("v1.0.0".to_string()),
                    sha: Some("sha123".to_string()),
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["gh-plugin"];
        match &p.source {
            LockedSource::Git { git, path } => {
                assert_eq!(git, "https://github.com/owner/repo.git");
                assert!(path.is_none());
            }
            other => panic!("expected Git source, got {:?}", other),
        }
        assert_eq!(p.git_commit.as_deref(), Some("sha123"));
    }

    #[test]
    fn resolve_plugin_github_source_no_sha_uses_context() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: gh-nosha
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  gh-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "gh-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitHub {
                    repo: "owner/repo".to_string(),
                    git_ref: Some("v1.0.0".to_string()),
                    sha: None,
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/owner/repo.git#v1.0.0".to_string(),
            "resolved_sha".to_string(),
        );

        let context = ResolveContext {
            registry_versions: rv,
            git_commits,
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["gh-plugin"];
        assert_eq!(p.git_commit.as_deref(), Some("resolved_sha"));
    }

    // --- resolve_plugin with GitUrl source_kind ---

    #[test]
    fn resolve_plugin_giturl_source_with_sha() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: giturl-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  giturl-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "giturl-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitUrl {
                    url: "https://gitlab.com/org/plugin.git".to_string(),
                    git_ref: None,
                    sha: Some("giturl_sha".to_string()),
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["giturl-plugin"];
        match &p.source {
            LockedSource::Git { git, .. } => {
                assert_eq!(git, "https://gitlab.com/org/plugin.git");
            }
            other => panic!("expected Git source, got {:?}", other),
        }
        assert_eq!(p.git_commit.as_deref(), Some("giturl_sha"));
    }

    #[test]
    fn resolve_plugin_giturl_source_no_sha() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: giturl-nosha
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  giturl-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "giturl-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitUrl {
                    url: "https://gitlab.com/org/plugin.git".to_string(),
                    git_ref: Some("main".to_string()),
                    sha: None,
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://gitlab.com/org/plugin.git#main".to_string(),
            "resolved_giturl".to_string(),
        );

        let context = ResolveContext {
            registry_versions: rv,
            git_commits,
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["giturl-plugin"];
        assert_eq!(p.git_commit.as_deref(), Some("resolved_giturl"));
    }

    // --- resolve_plugin with GitSubdir source_kind ---

    #[test]
    fn resolve_plugin_gitsubdir_source_with_sha() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: subdir-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  subdir-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "subdir-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitSubdir {
                    url: "owner/monorepo".to_string(),
                    path: "plugins/my-plugin".to_string(),
                    git_ref: None,
                    sha: Some("subdir_sha".to_string()),
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["subdir-plugin"];
        match &p.source {
            LockedSource::Git { git, path } => {
                // owner/monorepo should be expanded to full github URL
                assert_eq!(git, "https://github.com/owner/monorepo.git");
                assert_eq!(path.as_deref(), Some("plugins/my-plugin"));
            }
            other => panic!("expected Git source, got {:?}", other),
        }
        assert_eq!(p.git_commit.as_deref(), Some("subdir_sha"));
    }

    #[test]
    fn resolve_plugin_gitsubdir_no_sha_uses_context() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: subdir-nosha
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  subdir-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "subdir-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitSubdir {
                    url: "https://github.com/org/monorepo.git".to_string(),
                    path: "plugins/sub".to_string(),
                    git_ref: Some("v2.0".to_string()),
                    sha: None,
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/org/monorepo.git#v2.0".to_string(),
            "subdir_resolved".to_string(),
        );

        let context = ResolveContext {
            registry_versions: rv,
            git_commits,
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["subdir-plugin"];
        assert_eq!(p.git_commit.as_deref(), Some("subdir_resolved"));
    }

    // --- resolve_plugin with Npm source_kind ---

    #[test]
    fn resolve_plugin_npm_source() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: npm-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  npm-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "npm-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::Npm {
                    package: "@scope/my-plugin".to_string(),
                    version: Some("1.0.0".to_string()),
                    registry: Some("https://registry.npmjs.org".to_string()),
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["npm-plugin"];
        match &p.source {
            LockedSource::Npm {
                package,
                resolved_version,
                npm_registry,
            } => {
                assert_eq!(package, "@scope/my-plugin");
                assert_eq!(resolved_version.as_deref(), Some("1.0.0"));
                assert_eq!(npm_registry.as_deref(), Some("https://registry.npmjs.org"));
            }
            other => panic!("expected Npm source, got {:?}", other),
        }
        assert!(p.git_commit.is_none());
    }

    // --- resolve_plugin with RegistryLocal source_kind ---

    #[test]
    fn resolve_plugin_registry_local_source() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: reglocal-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  local-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "local-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "commit_abc".to_string(),
                path: Some("plugins/local-plugin".to_string()),
                source_kind: Some(PluginSourceKind::RegistryLocal {
                    path: "plugins/local-plugin".to_string(),
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["local-plugin"];
        match &p.source {
            LockedSource::Registry {
                registry,
                resolved_version,
                path,
            } => {
                assert_eq!(registry, "official");
                assert_eq!(resolved_version, "1.0.0");
                assert_eq!(path.as_deref(), Some("plugins/local-plugin"));
            }
            other => panic!("expected Registry source, got {:?}", other),
        }
        assert_eq!(p.git_commit.as_deref(), Some("commit_abc"));
    }

    // --- resolve_plugin invalid source ---

    #[test]
    fn resolve_plugin_invalid_source_errors() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: invalid-src
backends:
  claude-code: {}
plugins:
  bad-plugin:
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::InvalidSource { name, .. } => assert_eq!(name, "bad-plugin"),
            other => panic!("expected InvalidSource, got {:?}", other),
        }
    }

    // --- resolve_plugin registry not found ---

    #[test]
    fn resolve_plugin_registry_not_found() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: noreg
backends:
  claude-code: {}
registries:
  missing:
    url: https://example.com
plugins:
  some-plugin:
    registry: missing
    version: "^1"
"#,
        )
        .unwrap();

        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::RegistryNotFound { registry } => assert_eq!(registry, "missing"),
            other => panic!("expected RegistryNotFound, got {:?}", other),
        }
    }

    // --- resolve_plugin plugin not in registry ---

    #[test]
    fn resolve_plugin_not_in_registry() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: noplugin
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  ghost-plugin:
    registry: official
    version: "^1"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        rv.insert("official".to_string(), BTreeMap::new());

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::NoMatch { package, .. } => assert_eq!(package, "ghost-plugin"),
            other => panic!("expected NoMatch, got {:?}", other),
        }
    }

    // --- resolve_git_ref tests ---

    #[test]
    fn resolve_git_ref_pinned_commit() {
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let result = resolve_git_ref(
            "https://github.com/foo/bar.git",
            None,
            None,
            Some("deadbeef"),
            &context,
        )
        .unwrap();
        assert_eq!(result, "deadbeef");
    }

    #[test]
    fn resolve_git_ref_tag_lookup() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git#v1.0".to_string(),
            "tag_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result = resolve_git_ref(
            "https://github.com/foo/bar.git",
            Some("v1.0"),
            None,
            None,
            &context,
        )
        .unwrap();
        assert_eq!(result, "tag_sha");
    }

    #[test]
    fn resolve_git_ref_branch_lookup() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git#develop".to_string(),
            "branch_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result = resolve_git_ref(
            "https://github.com/foo/bar.git",
            None,
            Some("develop"),
            None,
            &context,
        )
        .unwrap();
        assert_eq!(result, "branch_sha");
    }

    #[test]
    fn resolve_git_ref_tag_takes_precedence_over_branch() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git#v1.0".to_string(),
            "tag_sha".to_string(),
        );
        git_commits.insert(
            "https://github.com/foo/bar.git#main".to_string(),
            "branch_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        // When both tag and branch are specified, tag wins (tag.or(branch))
        let result = resolve_git_ref(
            "https://github.com/foo/bar.git",
            Some("v1.0"),
            Some("main"),
            None,
            &context,
        )
        .unwrap();
        assert_eq!(result, "tag_sha");
    }

    #[test]
    fn resolve_git_ref_fallback_to_bare_url() {
        let mut git_commits = BTreeMap::new();
        // Only bare URL is in context (no #ref suffix).
        git_commits.insert(
            "https://github.com/foo/bar.git".to_string(),
            "bare_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result = resolve_git_ref(
            "https://github.com/foo/bar.git",
            None,
            None,
            None,
            &context,
        )
        .unwrap();
        assert_eq!(result, "bare_sha");
    }

    #[test]
    fn resolve_git_ref_not_resolved_error() {
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let err = resolve_git_ref(
            "https://github.com/foo/bar.git",
            Some("v999"),
            None,
            None,
            &context,
        )
        .unwrap_err();
        match err {
            ResolverError::GitRefNotResolved { url, git_ref } => {
                assert_eq!(url, "https://github.com/foo/bar.git");
                assert_eq!(git_ref, "v999");
            }
            other => panic!("expected GitRefNotResolved, got {:?}", other),
        }
    }

    // --- resolve_git_ref_from_context tests ---

    #[test]
    fn resolve_git_ref_from_context_with_ref() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git#v2.0".to_string(),
            "ctx_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result = resolve_git_ref_from_context(
            "https://github.com/foo/bar.git",
            Some("v2.0"),
            &context,
        )
        .unwrap();
        assert_eq!(result, "ctx_sha");
    }

    #[test]
    fn resolve_git_ref_from_context_no_ref_uses_head() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git#HEAD".to_string(),
            "head_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result =
            resolve_git_ref_from_context("https://github.com/foo/bar.git", None, &context)
                .unwrap();
        assert_eq!(result, "head_sha");
    }

    #[test]
    fn resolve_git_ref_from_context_fallback_to_bare_url() {
        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/foo/bar.git".to_string(),
            "bare_ctx_sha".to_string(),
        );
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let result =
            resolve_git_ref_from_context("https://github.com/foo/bar.git", None, &context)
                .unwrap();
        assert_eq!(result, "bare_ctx_sha");
    }

    #[test]
    fn resolve_git_ref_from_context_not_found_error() {
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        let err =
            resolve_git_ref_from_context("https://example.com/repo.git", Some("missing"), &context)
                .unwrap_err();
        match err {
            ResolverError::GitRefNotResolved { url, git_ref } => {
                assert_eq!(url, "https://example.com/repo.git");
                assert_eq!(git_ref, "missing");
            }
            other => panic!("expected GitRefNotResolved, got {:?}", other),
        }
    }

    // --- expand_source_url tests ---

    #[test]
    fn expand_source_url_github_shorthand() {
        assert_eq!(
            expand_source_url("owner/repo"),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn expand_source_url_full_url_unchanged() {
        let url = "https://gitlab.com/org/repo.git";
        assert_eq!(expand_source_url(url), url);
    }

    #[test]
    fn expand_source_url_ssh_unchanged() {
        let url = "git@github.com:owner/repo.git";
        assert_eq!(expand_source_url(url), url);
    }

    #[test]
    fn expand_source_url_nested_path_unchanged() {
        // More than one slash — not a GitHub shorthand.
        let url = "org/sub/repo";
        assert_eq!(expand_source_url(url), url);
    }

    #[test]
    fn expand_source_url_no_slash_unchanged() {
        // No slash at all.
        let url = "just-a-name";
        assert_eq!(expand_source_url(url), url);
    }

    // --- resolve_single for skills and agents ---

    #[test]
    fn resolve_single_skill() {
        let manifest = test_manifest();
        let context = test_context();
        let existing = Lockfile::new();

        let updated = resolve_single("local-skill", &manifest, &existing, &context).unwrap();
        assert_eq!(updated.skills.len(), 1);
        assert!(updated.skills.contains_key("local-skill"));
        match &updated.skills["local-skill"].source {
            LockedSource::Local { local } => assert_eq!(local, "./skills/local-skill"),
            other => panic!("expected Local source, got {:?}", other),
        }
    }

    #[test]
    fn resolve_single_agent() {
        let manifest = test_manifest();
        let context = test_context();
        let existing = Lockfile::new();

        let updated = resolve_single("my-reviewer", &manifest, &existing, &context).unwrap();
        assert_eq!(updated.agents.len(), 1);
        assert!(updated.agents.contains_key("my-reviewer"));
        match &updated.agents["my-reviewer"].source {
            LockedSource::Local { local } => assert_eq!(local, "./agents/my-reviewer.md"),
            other => panic!("expected Local source, got {:?}", other),
        }
    }

    // --- resolve_path_or_git_dep edge cases ---

    #[test]
    fn resolve_path_or_git_dep_invalid_source() {
        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits: BTreeMap::new(),
        };
        // Neither local nor git — should error.
        let err = resolve_path_or_git_dep(
            "bad-skill",
            "skill",
            false,
            "non-relative-path",
            None,       // no git URL
            None,
            None,
            None,
            &context,
        )
        .unwrap_err();
        match err {
            ResolverError::InvalidSource { name, reason } => {
                assert_eq!(name, "bad-skill");
                assert!(reason.contains("skill"));
            }
            other => panic!("expected InvalidSource, got {:?}", other),
        }
    }

    // --- resolve_plugin with branch (not tag) ---

    #[test]
    fn resolve_plugin_git_with_branch() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: branch-test
backends:
  claude-code: {}
plugins:
  branch-plugin:
    git: https://github.com/someone/plugin.git
    branch: develop
"#,
        )
        .unwrap();

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/someone/plugin.git#develop".to_string(),
            "branch_commit".to_string(),
        );

        let context = ResolveContext {
            registry_versions: BTreeMap::new(),
            git_commits,
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["branch-plugin"];
        assert_eq!(p.git_commit.as_deref(), Some("branch_commit"));
    }

    // --- conflict detection with skills ---

    #[test]
    fn conflict_detection_two_plugins_same_skill() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: skill-conflict
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  plugin-a:
    registry: official
    version: "^1.0.0"
  plugin-b:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let context = {
            let mut rv = BTreeMap::new();
            let mut official = BTreeMap::new();
            for name in &["plugin-a", "plugin-b"] {
                official.insert(
                    name.to_string(),
                    vec![PluginVersionInfo {
                        version: semver::Version::new(1, 0, 0),
                        git_commit: "x".to_string(),
                        path: None,
                        source_kind: None,
                    }],
                );
            }
            rv.insert("official".to_string(), official);
            ResolveContext {
                registry_versions: rv,
                git_commits: BTreeMap::new(),
            }
        };

        let mut plugin_assets = BTreeMap::new();
        plugin_assets.insert(
            "plugin-a".to_string(),
            PluginAssets {
                agents: vec![],
                skills: vec!["shared-skill".to_string()],
            },
        );
        plugin_assets.insert(
            "plugin-b".to_string(),
            PluginAssets {
                agents: vec![],
                skills: vec!["shared-skill".to_string()],
            },
        );

        let err = resolve_all(&manifest, &context, &plugin_assets).unwrap_err();
        match err {
            ResolverError::Conflict { name, sources } => {
                assert_eq!(name, "shared-skill");
                assert_eq!(sources.len(), 2);
            }
            other => panic!("expected Conflict error, got {:?}", other),
        }
    }

    // --- is_asset_included edge cases ---

    #[test]
    fn is_asset_included_no_filters() {
        assert!(is_asset_included("anything", &None, &None, AssetKind::Agent));
        assert!(is_asset_included("anything", &None, &None, AssetKind::Skill));
    }

    #[test]
    fn is_asset_included_include_empty_list() {
        // Empty include list means "include all" — the asset should be included.
        let filter = AssetFilter {
            agents: vec![],
            skills: vec![],
        };
        assert!(is_asset_included("anything", &Some(filter), &None, AssetKind::Agent));
    }

    #[test]
    fn is_asset_included_include_filters_agent() {
        let filter = AssetFilter {
            agents: vec!["allowed".to_string()],
            skills: vec![],
        };
        assert!(is_asset_included("allowed", &Some(filter.clone()), &None, AssetKind::Agent));
        assert!(!is_asset_included("denied", &Some(filter), &None, AssetKind::Agent));
    }

    #[test]
    fn is_asset_included_include_filters_skill() {
        let filter = AssetFilter {
            agents: vec![],
            skills: vec!["allowed-skill".to_string()],
        };
        assert!(is_asset_included("allowed-skill", &Some(filter.clone()), &None, AssetKind::Skill));
        assert!(!is_asset_included("other-skill", &Some(filter), &None, AssetKind::Skill));
    }

    #[test]
    fn is_asset_included_exclude_filters() {
        let filter = AssetFilter {
            agents: vec!["blocked".to_string()],
            skills: vec![],
        };
        assert!(!is_asset_included("blocked", &None, &Some(filter.clone()), AssetKind::Agent));
        assert!(is_asset_included("allowed", &None, &Some(filter), AssetKind::Agent));
    }

    // --- resolve_plugin with wildcard version constraint ---

    #[test]
    fn resolve_plugin_wildcard_version() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: wildcard-test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  any-version:
    registry: official
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "any-version".to_string(),
            vec![
                PluginVersionInfo {
                    version: semver::Version::new(1, 0, 0),
                    git_commit: "old".to_string(),
                    path: None,
                    source_kind: None,
                },
                PluginVersionInfo {
                    version: semver::Version::new(3, 0, 0),
                    git_commit: "newest".to_string(),
                    path: None,
                    source_kind: None,
                },
            ],
        );
        rv.insert("official".to_string(), official);

        let context = ResolveContext {
            registry_versions: rv,
            git_commits: BTreeMap::new(),
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["any-version"];
        match &p.source {
            LockedSource::Registry {
                resolved_version, ..
            } => {
                assert_eq!(resolved_version, "3.0.0");
            }
            other => panic!("expected Registry source, got {:?}", other),
        }
    }

    // --- detect_conflicts with missing plugin_assets (no asset info) ---

    #[test]
    fn detect_conflicts_missing_asset_info_skipped() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: no-assets
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  plugin-a:
    registry: official
    version: "^1"
"#,
        )
        .unwrap();

        // No asset info provided — should be skipped, no conflicts.
        let plugin_assets = BTreeMap::new();
        let result = detect_conflicts(&manifest, &plugin_assets);
        assert!(result.is_ok());
    }

    // --- resolve_single preserves existing entries ---

    #[test]
    fn resolve_single_preserves_existing_lockfile_entries() {
        let manifest = test_manifest();
        let context = test_context();

        // Pre-populate existing lockfile with one plugin
        let mut existing = Lockfile::new();
        existing.plugins.insert(
            "quality-review".to_string(),
            LockedEntry {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    resolved_version: "1.0.0".to_string(),
                    path: None,
                },
                git_commit: Some("aaa111".to_string()),
                integrity: None,
            },
        );

        // Resolve a skill — existing plugin should be preserved
        let updated = resolve_single("local-skill", &manifest, &existing, &context).unwrap();
        assert!(updated.plugins.contains_key("quality-review"));
        assert!(updated.skills.contains_key("local-skill"));
    }

    // --- GitHub source with no ref (uses HEAD) ---

    #[test]
    fn resolve_plugin_github_no_ref_uses_head() {
        use crate::sources::registry::PluginSourceKind;

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: gh-noref
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  gh-plugin:
    registry: official
    version: "^1.0.0"
"#,
        )
        .unwrap();

        let mut rv = BTreeMap::new();
        let mut official = BTreeMap::new();
        official.insert(
            "gh-plugin".to_string(),
            vec![PluginVersionInfo {
                version: semver::Version::new(1, 0, 0),
                git_commit: "ignored".to_string(),
                path: None,
                source_kind: Some(PluginSourceKind::GitHub {
                    repo: "owner/repo".to_string(),
                    git_ref: None,
                    sha: None,
                }),
            }],
        );
        rv.insert("official".to_string(), official);

        let mut git_commits = BTreeMap::new();
        git_commits.insert(
            "https://github.com/owner/repo.git#HEAD".to_string(),
            "head_commit".to_string(),
        );

        let context = ResolveContext {
            registry_versions: rv,
            git_commits,
        };
        let plugin_assets = BTreeMap::new();

        let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();
        let p = &lockfile.plugins["gh-plugin"];
        assert_eq!(p.git_commit.as_deref(), Some("head_commit"));
    }
}
