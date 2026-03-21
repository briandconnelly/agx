//! `agx lock` — resolve and write lockfile without downloading or generating.

use agx::cache;
use agx::config;
use agx::lockfile;
use agx::manifest;
use agx::resolver::{self, PluginVersionInfo, ResolveContext};
use agx::sources::registry::PluginSourceKind;
use agx::sources::{git, registry};
use anyhow::Context;
use clap::Args;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Args)]
pub struct LockArgs {}

/// Run the `agx lock` command.
///
/// Finds the manifest, fetches registry indexes and resolves git refs,
/// then produces a lockfile from the resolve pipeline.
pub fn run(_args: LockArgs) -> anyhow::Result<()> {
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
        .expect("manifest must have a parent directory");

    let manifest = manifest::load(&manifest_path)?;
    let lockfile = run_lock(&manifest, project_root)?;

    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    lockfile::save(&lockfile, &lockfile_path)?;

    eprintln!("lockfile written to {}", lockfile_path.display());
    Ok(())
}

/// Core lock logic: build a ResolveContext from fetched data, then resolve.
///
/// This is extracted so that `agx install` and `agx add`/`agx remove`
/// can reuse the lock step without going through the CLI entry point.
pub fn run_lock(
    manifest: &agx::manifest::types::AgentEnvironment,
    project_root: &Path,
) -> anyhow::Result<agx::lockfile::types::Lockfile> {
    let _ = project_root; // project_root reserved for future use (e.g. local path validation)

    let context = build_resolve_context(manifest)?;

    // For now, we pass an empty plugin_assets map. Full plugin asset
    // introspection (reading asset manifests from downloaded plugins)
    // requires fetching the plugin content, which is done at install time.
    let plugin_assets = BTreeMap::new();

    let lockfile = resolver::resolve_all(manifest, &context, &plugin_assets)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(lockfile)
}

/// Build a [`ResolveContext`] by fetching registry indexes and resolving git refs.
fn build_resolve_context(
    manifest: &agx::manifest::types::AgentEnvironment,
) -> anyhow::Result<ResolveContext> {
    let mut registry_versions: BTreeMap<String, BTreeMap<String, Vec<PluginVersionInfo>>> =
        BTreeMap::new();
    let mut git_commits: BTreeMap<String, String> = BTreeMap::new();

    let git_cache = cache::git_dir()?;
    let reg_cache = cache::registry_dir()?;

    // 1. Fetch registry indexes
    for (reg_name, reg_config) in &manifest.registries {
        if config::is_offline() {
            let reg_dir = reg_cache.join(git::url_to_slug(&reg_config.url));
            if !reg_dir.exists() {
                anyhow::bail!(
                    "network operations disabled (AGX_OFFLINE=1) and registry '{reg_name}' is not cached; \
                     ensure all registries are cached or unset AGX_OFFLINE"
                );
            }
        }
        eprintln!("fetching registry '{reg_name}'...");
        let reg_dir = reg_cache.join(git::url_to_slug(&reg_config.url));
        let pin = reg_config.pin.as_deref();
        let index = registry::fetch_index_pinned(&reg_config.url, &reg_dir, pin)
            .with_context(|| format!("fetching registry '{reg_name}'"))?;

        let mut plugin_versions_map: BTreeMap<String, Vec<PluginVersionInfo>> = BTreeMap::new();
        for (plugin_name, entry) in &index.plugins {
            let versions: Vec<PluginVersionInfo> = entry
                .versions
                .iter()
                .filter_map(|(ver_str, detail)| {
                    let version = semver::Version::parse(ver_str).ok()?;
                    Some(PluginVersionInfo {
                        version,
                        git_commit: detail.commit.clone(),
                        path: detail.path.clone(),
                        source_kind: detail.source_kind.clone(),
                    })
                })
                .collect();
            plugin_versions_map.insert(plugin_name.clone(), versions);
        }
        registry_versions.insert(reg_name.clone(), plugin_versions_map);
    }

    // 2. Resolve git refs for all git-sourced dependencies
    // Collect all unique git URLs and their refs
    let mut git_refs_to_resolve: Vec<(String, Option<String>)> = Vec::new();

    // 2a. Collect git URLs from registry plugin source kinds
    // (GitHub, GitUrl, GitSubdir plugins that live outside the registry repo)
    for plugin_map in registry_versions.values() {
        for versions in plugin_map.values() {
            for info in versions {
                if let Some(ref sk) = info.source_kind {
                    match sk {
                        PluginSourceKind::GitHub { repo, git_ref, sha } => {
                            if sha.is_none() {
                                let url = format!("https://github.com/{}.git", repo);
                                git_refs_to_resolve.push((url, git_ref.clone()));
                            }
                        }
                        PluginSourceKind::GitUrl { url, git_ref, sha } => {
                            if sha.is_none() {
                                git_refs_to_resolve.push((url.clone(), git_ref.clone()));
                            }
                        }
                        PluginSourceKind::GitSubdir { url, git_ref, sha, .. } => {
                            if sha.is_none() {
                                let expanded = expand_git_url(url);
                                git_refs_to_resolve.push((expanded, git_ref.clone()));
                            }
                        }
                        PluginSourceKind::RegistryLocal { .. }
                        | PluginSourceKind::Npm { .. } => {}
                    }
                }
            }
        }
    }

    // 2b. Collect git URLs from manifest-level dependencies
    for dep in manifest.plugins.values() {
        if let Some(ref url) = dep.git {
            let git_ref = dep
                .tag
                .as_deref()
                .or(dep.branch.as_deref())
                .map(|s| s.to_string());
            git_refs_to_resolve.push((url.clone(), git_ref));
        }
    }

    for dep in manifest.skills.values() {
        if let Some(ref url) = dep.git {
            let git_ref = dep
                .tag
                .as_deref()
                .or(dep.branch.as_deref())
                .map(|s| s.to_string());
            git_refs_to_resolve.push((url.clone(), git_ref));
        }
    }

    for dep in manifest.agents.values() {
        if let Some(ref url) = dep.git {
            let git_ref = dep
                .tag
                .as_deref()
                .or(dep.branch.as_deref())
                .map(|s| s.to_string());
            git_refs_to_resolve.push((url.clone(), git_ref));
        }
    }

    // Deduplicate by (url, ref) pair
    git_refs_to_resolve.sort();
    git_refs_to_resolve.dedup();

    for (url, git_ref) in &git_refs_to_resolve {
        let slug = git::url_to_slug(url);
        let repo_path = git_cache.join(&slug);

        // Clone or fetch the bare repo
        if repo_path.exists() {
            if !config::is_offline() {
                eprintln!("updating git repo {slug}...");
                git::fetch(&repo_path)
                    .with_context(|| format!("fetching git repo '{url}'"))?;
            }
        } else {
            if config::is_offline() {
                anyhow::bail!(
                    "network operations disabled (AGX_OFFLINE=1) and git repo '{url}' is not cached; \
                     ensure all dependencies are cached or unset AGX_OFFLINE"
                );
            }
            eprintln!("cloning git repo {slug}...");
            std::fs::create_dir_all(&git_cache)
                .context("creating git cache directory")?;
            git::clone(url, &repo_path)
                .with_context(|| format!("cloning git repo '{url}'"))?;
        }

        // Resolve the ref to a SHA
        let sha = if let Some(r) = git_ref {
            git::resolve_ref(&repo_path, r)
                .with_context(|| format!("resolving ref '{r}' in '{url}'"))?
        } else {
            git::resolve_default_branch(&repo_path)
                .with_context(|| format!("resolving HEAD in '{url}'"))?
        };

        // Store both the compound key and the bare URL key
        let ref_str = git_ref.as_deref().unwrap_or("HEAD");
        let lookup_key = format!("{}#{}", url, ref_str);
        git_commits.insert(lookup_key, sha.clone());
        // Also store bare URL for fallback
        git_commits.entry(url.clone()).or_insert(sha);
    }

    Ok(ResolveContext {
        registry_versions,
        git_commits,
    })
}

/// Expand a git URL that might be a GitHub `owner/repo` shorthand.
fn expand_git_url(url: &str) -> String {
    // If it looks like a shorthand (no protocol, no dots before /), expand to https
    if !url.contains("://") && !url.contains('@') && url.matches('/').count() == 1 {
        format!("https://github.com/{}.git", url)
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::lockfile::types::LockedSource;
    use agx::manifest::types::AgentEnvironment;

    /// Test that `run_lock` produces a valid lockfile for a manifest with only
    /// local dependencies (no network access required).
    #[test]
    fn lock_local_only_deps() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: local-test
backends:
  claude-code: {}
skills:
  deploy-checklist:
    path: ./skills/deploy-checklist
agents:
  my-reviewer:
    path: ./agents/my-reviewer.md
    model: opus
"#,
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lockfile = run_lock(&manifest, dir.path()).unwrap();

        // Should have both entries
        assert_eq!(lockfile.skills.len(), 1);
        assert_eq!(lockfile.agents.len(), 1);
        assert!(lockfile.plugins.is_empty());

        // Skill should be local source
        let skill = &lockfile.skills["deploy-checklist"];
        match &skill.source {
            LockedSource::Local { local } => {
                assert_eq!(local, "./skills/deploy-checklist");
            }
            other => panic!("expected Local source, got {:?}", other),
        }
        assert!(skill.git_commit.is_none());

        // Agent should be local source
        let agent = &lockfile.agents["my-reviewer"];
        match &agent.source {
            LockedSource::Local { local } => {
                assert_eq!(local, "./agents/my-reviewer.md");
            }
            other => panic!("expected Local source, got {:?}", other),
        }
        assert!(agent.git_commit.is_none());
    }

    /// Test that `run_lock` produces a lockfile with correct metadata.
    #[test]
    fn lock_lockfile_metadata() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            "name: meta-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lockfile = run_lock(&manifest, dir.path()).unwrap();

        assert_eq!(lockfile.lockfile_version, 1);
        assert_eq!(lockfile.agx_version, env!("CARGO_PKG_VERSION"));
        assert!(!lockfile.generated_at.is_empty());
    }

    /// Test that `run_lock` with an empty manifest produces an empty lockfile.
    #[test]
    fn lock_empty_manifest() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            "name: empty\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lockfile = run_lock(&manifest, dir.path()).unwrap();

        assert!(lockfile.plugins.is_empty());
        assert!(lockfile.skills.is_empty());
        assert!(lockfile.agents.is_empty());
    }

    /// Test that `run_lock` with mixed local deps produces correct sources.
    #[test]
    fn lock_mixed_local_deps() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: mixed-local
backends:
  claude-code: {}
  gemini-cli: {}
skills:
  skill-a:
    path: ./skills/a
  skill-b:
    path: ./skills/b
agents:
  agent-x:
    path: ./agents/x.md
  agent-y:
    path: ../shared/agents/y.md
"#,
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lockfile = run_lock(&manifest, dir.path()).unwrap();

        assert_eq!(lockfile.skills.len(), 2);
        assert_eq!(lockfile.agents.len(), 2);

        // Verify all are local sources
        for (_name, skill) in &lockfile.skills {
            assert!(matches!(skill.source, LockedSource::Local { .. }));
        }
        for (_name, agent) in &lockfile.agents {
            assert!(matches!(agent.source, LockedSource::Local { .. }));
        }
    }
}
