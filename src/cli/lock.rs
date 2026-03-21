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
pub fn build_resolve_context(
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
pub fn expand_git_url(url: &str) -> String {
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
    use agx::manifest::types::{
        AgentDependency, AgentEnvironment, PluginDependency, SkillDependency,
    };

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

    // --- expand_git_url tests ---

    /// Shorthand `owner/repo` should expand to full GitHub HTTPS URL.
    #[test]
    fn expand_git_url_shorthand() {
        assert_eq!(
            expand_git_url("anthropics/claude-plugins"),
            "https://github.com/anthropics/claude-plugins.git"
        );
    }

    /// Full HTTPS URL should pass through unchanged.
    #[test]
    fn expand_git_url_https_passthrough() {
        let url = "https://github.com/anthropics/claude-plugins.git";
        assert_eq!(expand_git_url(url), url);
    }

    /// SSH URL should pass through unchanged (contains `@`).
    #[test]
    fn expand_git_url_ssh_passthrough() {
        let url = "git@github.com:anthropics/claude-plugins.git";
        assert_eq!(expand_git_url(url), url);
    }

    /// URL with `.git` suffix and protocol should pass through.
    #[test]
    fn expand_git_url_git_protocol_passthrough() {
        let url = "git://github.com/anthropics/claude-plugins.git";
        assert_eq!(expand_git_url(url), url);
    }

    /// HTTP URL should pass through unchanged.
    #[test]
    fn expand_git_url_http_passthrough() {
        let url = "http://example.com/repo.git";
        assert_eq!(expand_git_url(url), url);
    }

    /// A path with multiple slashes should not be treated as shorthand.
    #[test]
    fn expand_git_url_deep_path_not_shorthand() {
        let url = "org/sub/repo";
        // Has 2 slashes, not exactly 1, so passes through
        assert_eq!(expand_git_url(url), url);
    }

    /// A single word with no slashes should not be treated as shorthand.
    #[test]
    fn expand_git_url_no_slash_passthrough() {
        let url = "just-a-name";
        assert_eq!(expand_git_url(url), url);
    }

    // --- build_resolve_context tests ---

    /// `build_resolve_context` with a local-only manifest should return empty context.
    #[test]
    fn build_resolve_context_local_only() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // Point the cache at a temp dir so we don't pollute real cache
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: local-ctx-test
backends:
  claude-code: {}
skills:
  my-skill:
    path: ./skills/my-skill
agents:
  my-agent:
    path: ./agents/my-agent.md
"#,
        )
        .unwrap();

        let ctx = build_resolve_context(&manifest).unwrap();
        assert!(ctx.registry_versions.is_empty());
        assert!(ctx.git_commits.is_empty());

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// `build_resolve_context` with an empty manifest returns empty context.
    #[test]
    fn build_resolve_context_empty_manifest() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };

        let manifest: AgentEnvironment = serde_yaml::from_str(
            "name: empty\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let ctx = build_resolve_context(&manifest).unwrap();
        assert!(ctx.registry_versions.is_empty());
        assert!(ctx.git_commits.is_empty());

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// `build_resolve_context` with git deps pointing to a local bare repo.
    #[test]
    fn build_resolve_context_with_git_deps() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        // Create a bare repo with a commit
        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        // Initialize a normal repo, add a commit, then clone --bare
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        // Use the bare repo path as a file:// URL for a git dep
        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: git-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "remote-skill".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/remote".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        let ctx = build_resolve_context(&manifest).unwrap();
        // Should have resolved a git commit
        assert!(!ctx.git_commits.is_empty());
        // Should have an entry for the URL
        assert!(ctx.git_commits.contains_key(&repo_url));
        // The resolved commit should be a 40-char hex SHA
        let sha = &ctx.git_commits[&repo_url];
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// Dedup logic: duplicate git URLs should only clone once and produce the same SHA.
    #[test]
    fn build_resolve_context_dedup_git_urls() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        // Create a bare repo with a commit
        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        // Manifest with two deps pointing to the same git URL
        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: dedup-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "skill-a".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/a".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        manifest.skills.insert(
            "skill-b".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/b".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        manifest.agents.insert(
            "agent-c".to_string(),
            AgentDependency {
                git: Some(repo_url.clone()),
                path: "agents/c.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );

        let ctx = build_resolve_context(&manifest).unwrap();
        // Despite 3 deps using the same URL, there should be only one clone
        // and the SHA should be consistent
        assert!(ctx.git_commits.contains_key(&repo_url));
        let sha = &ctx.git_commits[&repo_url];
        assert_eq!(sha.len(), 40);

        // Check that the compound key also exists (url#HEAD)
        let head_key = format!("{}#HEAD", &repo_url);
        assert!(ctx.git_commits.contains_key(&head_key));
        assert_eq!(&ctx.git_commits[&head_key], sha);

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// `build_resolve_context` with git dep using a tag ref.
    #[test]
    fn build_resolve_context_git_with_tag() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        // Create a repo with a tag
        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["tag", "v1.0.0"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: tag-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "tagged-skill".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/tagged".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
            },
        );

        let ctx = build_resolve_context(&manifest).unwrap();
        // Should have a compound key with the tag
        let tag_key = format!("{}#v1.0.0", &repo_url);
        assert!(ctx.git_commits.contains_key(&tag_key));
        let sha = &ctx.git_commits[&tag_key];
        assert_eq!(sha.len(), 40);

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// `build_resolve_context` with git dep using a branch ref.
    #[test]
    fn build_resolve_context_git_with_branch() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: branch-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.agents.insert(
            "branch-agent".to_string(),
            AgentDependency {
                git: Some(repo_url.clone()),
                path: "agents/branch.md".to_string(),
                tag: None,
                branch: Some("main".to_string()),
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );

        let ctx = build_resolve_context(&manifest).unwrap();
        let branch_key = format!("{}#main", &repo_url);
        assert!(ctx.git_commits.contains_key(&branch_key));
        let sha = &ctx.git_commits[&branch_key];
        assert_eq!(sha.len(), 40);

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// `build_resolve_context` with plugin git deps.
    #[test]
    fn build_resolve_context_plugin_git_dep() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: plugin-git-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.plugins.insert(
            "my-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: Some(repo_url.clone()),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );

        // The tag doesn't exist in the bare repo, so this should error
        // when trying to resolve the ref. But the clone should succeed.
        // Actually, let's create the tag first.
        std::process::Command::new("git")
            .args(["tag", "v1.0.0"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        // Re-clone the bare repo with the tag
        std::fs::remove_dir_all(&bare_dir).unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let ctx = build_resolve_context(&manifest).unwrap();
        let key = format!("{}#v1.0.0", &repo_url);
        assert!(ctx.git_commits.contains_key(&key));

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    /// Test offline mode error for uncached git repos.
    #[test]
    fn build_resolve_context_offline_git_error() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };
        unsafe { std::env::set_var("AGX_OFFLINE", "1") };

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: offline-git-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "remote-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/nonexistent/repo.git".to_string()),
                path: "skills/remote".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        let result = build_resolve_context(&manifest);
        unsafe { std::env::remove_var("AGX_OFFLINE") };
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.as_ref().err().unwrap());
        assert!(err_msg.contains("AGX_OFFLINE") || err_msg.contains("not cached"));
    }

    /// Test offline mode error for uncached registries.
    #[test]
    fn build_resolve_context_offline_registry_error() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };
        unsafe { std::env::set_var("AGX_OFFLINE", "1") };

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: offline-reg-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.registries.insert(
            "official".to_string(),
            agx::manifest::types::RegistryConfig {
                url: "https://registry.example.com/index.git".to_string(),
                auth: None,
                pin: None,
            },
        );

        let result = build_resolve_context(&manifest);
        unsafe { std::env::remove_var("AGX_OFFLINE") };
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.as_ref().err().unwrap());
        assert!(err_msg.contains("AGX_OFFLINE") || err_msg.contains("not cached"));
    }

    /// Test that git dep that already exists in cache skips clone in offline mode.
    #[test]
    fn build_resolve_context_offline_cached_git_succeeds() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        // First, create a bare repo and populate cache while online
        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: offline-cached-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "cached-skill".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/cached".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        // First build populates cache
        let ctx = build_resolve_context(&manifest).unwrap();
        assert!(!ctx.git_commits.is_empty());

        // Now go offline — should succeed because repo is cached
        unsafe { std::env::set_var("AGX_OFFLINE", "1") };
        let ctx2 = build_resolve_context(&manifest).unwrap();
        unsafe { std::env::remove_var("AGX_OFFLINE") };
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(!ctx2.git_commits.is_empty());
    }

    /// Test run_lock with multiple local skills and agents.
    #[test]
    fn lock_multiple_local_skills() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: multi-skill
backends:
  claude-code: {}
skills:
  skill-1:
    path: ./skills/one
  skill-2:
    path: ./skills/two
  skill-3:
    path: ./skills/three
"#,
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let lockfile = run_lock(&manifest, dir.path()).unwrap();
        assert_eq!(lockfile.skills.len(), 3);
        for name in &["skill-1", "skill-2", "skill-3"] {
            assert!(lockfile.skills.contains_key(*name));
            assert!(matches!(lockfile.skills[*name].source, LockedSource::Local { .. }));
        }
    }

    /// Test expand_git_url with edge cases.
    #[test]
    fn expand_git_url_empty_string() {
        assert_eq!(expand_git_url(""), "");
    }

    /// Test that tag ref takes precedence over branch ref in git deps.
    #[test]
    fn build_resolve_context_tag_over_branch() {
        // When both tag and branch are set, tag should be used (tag is checked first
        // in the `dep.tag.as_deref().or(dep.branch.as_deref())` chain)
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        unsafe { std::env::set_var("AGX_CACHE_DIR", &cache_dir) };

        let work_dir = tmp.path().join("work");
        let bare_dir = tmp.path().join("bare.git");
        std::fs::create_dir_all(&work_dir).unwrap();

        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "--allow-empty", "-m", "init"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["tag", "v2.0.0"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&work_dir)
            .arg(&bare_dir)
            .output()
            .unwrap();

        let repo_url = format!("file://{}", bare_dir.display());

        let mut manifest: AgentEnvironment = serde_yaml::from_str(
            "name: precedence-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        manifest.skills.insert(
            "prec-skill".to_string(),
            SkillDependency {
                git: Some(repo_url.clone()),
                path: "skills/prec".to_string(),
                tag: Some("v2.0.0".to_string()),
                branch: Some("main".to_string()),
                commit: None,
            },
        );

        let ctx = build_resolve_context(&manifest).unwrap();
        // The tag key should be used, not the branch key
        let tag_key = format!("{}#v2.0.0", &repo_url);
        assert!(ctx.git_commits.contains_key(&tag_key));

        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }
}
