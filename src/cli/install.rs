//! `agx install` — resolve, download, and generate backend configs.

use agx::backends::{get_backend, ResolvedEnvironment};
use agx::cache;
use agx::config;
use agx::context;
use agx::hooks;
use agx::lockfile;
use agx::lockfile::types::{LockedSource, Lockfile};
use agx::manifest;
use agx::manifest::types::AgentEnvironment;
use agx::sources::git;
use anyhow::Context;
use clap::Args;
use std::path::{Path, PathBuf};

use super::lock::run_lock;

#[derive(Debug, Args)]
pub struct InstallArgs {}

/// Run the `agx install` command.
///
/// If no lockfile exists or it's stale, runs the lock step first.
/// Then fetches all remote assets to the cache and runs generate
/// for all backends.
pub fn run(_args: InstallArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in {} or parent directories",
            config::MANIFEST_FILE,
            cwd.display()
        )
    })?;

    run_install_at(&manifest_path)
}

/// Run install for a project at a given manifest path.
///
/// This is called by `agx install` and also by `agx init` after
/// scaffolding a new project.
pub fn run_install_at(manifest_path: &Path) -> anyhow::Result<()> {
    let project_root = manifest_path
        .parent()
        .expect("manifest must have a parent directory")
        .to_path_buf();

    let manifest = manifest::load(manifest_path)?;

    // Run pre-install hooks
    let hook_cfg = manifest.hooks.as_ref();
    let pre_install = hook_cfg.map(|h| h.pre_install.as_slice()).unwrap_or_default();
    hooks::run_hooks("pre-install", pre_install)?;

    // Load or generate lockfile
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    let lockfile = load_or_lock(&manifest, &project_root, &lockfile_path)?;

    // Fetch all remote assets to cache
    fetch_assets(&manifest, &lockfile)?;

    // Run generate for all backends
    run_generate(&manifest, &lockfile, &project_root)?;

    // Generate context files
    context::generate_context_files(&manifest, &project_root)
        .context("generating context files")?;

    // Run post-install hooks
    let post_install = hook_cfg.map(|h| h.post_install.as_slice()).unwrap_or_default();
    hooks::run_hooks("post-install", post_install)?;

    eprintln!("install complete.");
    Ok(())
}

/// Load the lockfile if it exists and is fresh, otherwise run lock.
fn load_or_lock(
    manifest: &AgentEnvironment,
    project_root: &Path,
    lockfile_path: &Path,
) -> anyhow::Result<Lockfile> {
    if lockfile_path.exists() {
        let lockfile = lockfile::load(lockfile_path)?;
        if lockfile::is_stale(&lockfile, manifest) {
            eprintln!("lockfile is stale, re-resolving...");
            let lockfile = run_lock(manifest, project_root)?;
            lockfile::save(&lockfile, lockfile_path)?;
            Ok(lockfile)
        } else {
            Ok(lockfile)
        }
    } else {
        eprintln!("no lockfile found, resolving...");
        let lockfile = run_lock(manifest, project_root)?;
        lockfile::save(&lockfile, lockfile_path)?;
        Ok(lockfile)
    }
}

/// Fetch all remote assets referenced in the lockfile to the global cache.
fn fetch_assets(manifest: &AgentEnvironment, lockfile: &Lockfile) -> anyhow::Result<()> {
    let git_cache = cache::git_dir()?;

    // Fetch plugins
    for (name, entry) in &lockfile.plugins {
        match &entry.source {
            LockedSource::Registry {
                registry,
                resolved_version,
                path,
            } => {
                if cache::get_plugin(name, resolved_version)?.is_some() {
                    // Already cached — still verify integrity
                    if let Some(ref expected_hash) = entry.integrity {
                        let cached_path = cache::get_plugin(name, resolved_version)?;
                        if let Some(ref p) = cached_path {
                            verify_cached_integrity(name, p, expected_hash)?;
                        }
                    }
                    continue;
                }
                if let Some(ref commit) = entry.git_commit {
                    // Look up the registry URL from the manifest to find the bare repo
                    let reg_url = manifest
                        .registries
                        .get(registry)
                        .map(|r| r.url.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "registry '{}' referenced in lockfile but not found in manifest",
                                registry
                            )
                        })?;
                    eprintln!("fetching plugin '{name}@{resolved_version}'...");
                    let reg_cache = cache::registry_dir()?;
                    extract_from_git_cache(
                        &reg_cache,
                        reg_url,
                        commit,
                        path.as_deref(),
                        &|src| cache::store_plugin(name, resolved_version, src),
                    )?;
                }
                // Verify integrity after fetching
                if let Some(ref expected_hash) = entry.integrity {
                    if let Some(ref p) = cache::get_plugin(name, resolved_version)? {
                        verify_cached_integrity(name, p, expected_hash)?;
                    }
                }
            }
            LockedSource::Git { git: url, path } => {
                if let Some(ref commit) = entry.git_commit {
                    // Use commit as version key for git plugins
                    if cache::get_plugin(name, commit)?.is_some() {
                        // Already cached — still verify integrity
                        if let Some(ref expected_hash) = entry.integrity {
                            if let Some(ref p) = cache::get_plugin(name, commit)? {
                                verify_cached_integrity(name, p, expected_hash)?;
                            }
                        }
                        continue;
                    }
                    eprintln!("fetching plugin '{name}'...");
                    extract_from_git_cache(
                        &git_cache,
                        url,
                        commit,
                        path.as_deref(),
                        &|src| cache::store_plugin(name, commit, src),
                    )?;
                    // Verify integrity after fetching
                    if let Some(ref expected_hash) = entry.integrity {
                        if let Some(ref p) = cache::get_plugin(name, commit)? {
                            verify_cached_integrity(name, p, expected_hash)?;
                        }
                    }
                }
            }
            LockedSource::Npm { package, .. } => {
                anyhow::bail!(
                    "npm source '{}' for plugin '{}' is not yet supported; \
                     install the plugin from git instead",
                    package,
                    name
                );
            }
            LockedSource::Local { .. } => {} // Local assets are read at generate time
        }
    }

    // Fetch skills
    for (name, entry) in &lockfile.skills {
        match &entry.source {
            LockedSource::Git { git: url, path } => {
                if let Some(ref commit) = entry.git_commit {
                    if cache::get_skill(name, commit)?.is_some() {
                        // Already cached — still verify integrity
                        if let Some(ref expected_hash) = entry.integrity {
                            if let Some(ref p) = cache::get_skill(name, commit)? {
                                verify_cached_integrity(name, p, expected_hash)?;
                            }
                        }
                        continue;
                    }
                    eprintln!("fetching skill '{name}'...");
                    extract_from_git_cache(
                        &git_cache,
                        url,
                        commit,
                        path.as_deref(),
                        &|src| cache::store_skill(name, commit, src),
                    )?;
                    // Verify integrity after fetching
                    if let Some(ref expected_hash) = entry.integrity {
                        if let Some(ref p) = cache::get_skill(name, commit)? {
                            verify_cached_integrity(name, p, expected_hash)?;
                        }
                    }
                }
            }
            LockedSource::Local { .. } => {}
            _ => {}
        }
    }

    // Fetch agents
    for (name, entry) in &lockfile.agents {
        match &entry.source {
            LockedSource::Git { git: url, path } => {
                if let Some(ref commit) = entry.git_commit {
                    if cache::get_agent(name, commit)?.is_some() {
                        // Already cached — still verify integrity
                        if let Some(ref expected_hash) = entry.integrity {
                            if let Some(ref p) = cache::get_agent(name, commit)? {
                                verify_cached_integrity(name, p, expected_hash)?;
                            }
                        }
                        continue;
                    }
                    eprintln!("fetching agent '{name}'...");
                    extract_from_git_cache(
                        &git_cache,
                        url,
                        commit,
                        path.as_deref(),
                        &|src| cache::store_agent(name, commit, src),
                    )?;
                    // Verify integrity after fetching
                    if let Some(ref expected_hash) = entry.integrity {
                        if let Some(ref p) = cache::get_agent(name, commit)? {
                            verify_cached_integrity(name, p, expected_hash)?;
                        }
                    }
                }
            }
            LockedSource::Local { .. } => {}
            _ => {}
        }
    }

    Ok(())
}

/// Extract content from a cached bare git repo and store it via the provided closure.
fn extract_from_git_cache(
    git_cache: &Path,
    url_or_name: &str,
    commit: &str,
    subpath: Option<&str>,
    store_fn: &dyn Fn(&Path) -> anyhow::Result<PathBuf>,
) -> anyhow::Result<()> {
    let slug = git::url_to_slug(url_or_name);
    let repo_path = git_cache.join(&slug);

    if !repo_path.exists() {
        // The bare repo should have been cloned during lock, but if the
        // cache was cleaned, we need to re-clone. We don't have the URL
        // handy when url_or_name is a name, so best-effort.
        if config::is_offline() {
            anyhow::bail!(
                "git cache missing for '{}' and network operations are disabled (AGX_OFFLINE=1); \
                 ensure all dependencies are cached or unset AGX_OFFLINE",
                url_or_name
            );
        }
        anyhow::bail!(
            "git cache missing for '{}'; run `agx lock` to re-fetch",
            url_or_name
        );
    }

    let tmp = tempfile::tempdir().context("creating temp directory")?;
    let extract_path = subpath.unwrap_or(".");

    git::extract(&repo_path, commit, extract_path, tmp.path())
        .with_context(|| format!("extracting '{}' at commit {}", extract_path, commit))?;

    // If we extracted a subpath, the content is under tmp/subpath
    let source_dir = if let Some(sp) = subpath {
        tmp.path().join(sp)
    } else {
        tmp.path().to_path_buf()
    };

    store_fn(&source_dir)?;
    Ok(())
}

/// Verify integrity of a cached asset by computing a SHA-256 hash over its
/// directory contents and comparing against the expected hash from the lockfile.
///
/// Returns an error if the computed hash does not match, indicating possible
/// corruption or tampering.
fn verify_cached_integrity(name: &str, cached_path: &Path, expected: &str) -> anyhow::Result<()> {
    let actual = cache::integrity::hash_dir(cached_path)
        .with_context(|| format!("computing integrity hash for '{name}'"))?;
    if actual != expected {
        anyhow::bail!(
            "integrity check failed for '{}': expected {}, got {}",
            name,
            expected,
            actual
        );
    }
    Ok(())
}

/// Run backend generation for all declared backends.
fn run_generate(
    manifest: &AgentEnvironment,
    lockfile: &Lockfile,
    project_root: &Path,
) -> anyhow::Result<()> {
    let cache_dir = cache::cache_dir()?;
    let resolved = ResolvedEnvironment {
        manifest,
        lockfile,
        cache_dir: &cache_dir,
    };

    for backend_name in manifest.backends.keys() {
        let backend = get_backend(backend_name).ok_or_else(|| {
            anyhow::anyhow!(
                "no built-in backend for '{}'; available: claude-code, gemini-cli, generic",
                backend_name
            )
        })?;

        eprintln!("generating {} backend...", backend.name());
        backend
            .generate(&resolved, project_root)
            .with_context(|| format!("generating {} backend", backend.name()))?;
    }

    Ok(())
}
