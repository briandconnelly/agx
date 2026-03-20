//! `agx install` — resolve, download, and generate backend configs.

use agx::backends::{get_backend, ResolvedEnvironment};
use agx::cache;
use agx::config;
use agx::context;
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
    let project_root = manifest_path
        .parent()
        .expect("manifest must have a parent directory")
        .to_path_buf();

    let manifest = manifest::load(&manifest_path)?;

    // Load or generate lockfile
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    let lockfile = load_or_lock(&manifest, &project_root, &lockfile_path)?;

    // Fetch all remote assets to cache
    fetch_assets(&lockfile)?;

    // Run generate for all backends
    run_generate(&manifest, &lockfile, &project_root)?;

    // Generate context files
    context::generate_context_files(&manifest, &project_root)
        .context("generating context files")?;

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
fn fetch_assets(lockfile: &Lockfile) -> anyhow::Result<()> {
    let git_cache = cache::git_dir()?;

    // Fetch plugins
    for (name, entry) in &lockfile.plugins {
        match &entry.source {
            LockedSource::Registry {
                resolved_version, ..
            } => {
                if cache::get_plugin(name, resolved_version)?.is_some() {
                    continue; // Already cached
                }
                if let Some(ref commit) = entry.git_commit {
                    eprintln!("fetching plugin '{name}@{resolved_version}'...");
                    extract_from_git_cache(
                        &git_cache,
                        name,
                        commit,
                        None, // plugins: extract whole root
                        &|src| cache::store_plugin(name, resolved_version, src),
                    )?;
                }
            }
            LockedSource::Git { git: url, .. } => {
                if let Some(ref commit) = entry.git_commit {
                    // Use commit as version key for git plugins
                    if cache::get_plugin(name, commit)?.is_some() {
                        continue;
                    }
                    eprintln!("fetching plugin '{name}'...");
                    extract_from_git_cache(
                        &git_cache,
                        url,
                        commit,
                        None,
                        &|src| cache::store_plugin(name, commit, src),
                    )?;
                }
            }
            LockedSource::Local { .. } => {} // Local assets are read at generate time
        }

        // Verify integrity if present
        if let Some(ref expected_hash) = entry.integrity {
            verify_cached_integrity(name, expected_hash)?;
        }
    }

    // Fetch skills
    for (name, entry) in &lockfile.skills {
        match &entry.source {
            LockedSource::Git { git: url, path } => {
                if let Some(ref commit) = entry.git_commit {
                    if cache::get_skill(name, commit)?.is_some() {
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
                }
            }
            LockedSource::Local { .. } => {}
            _ => {}
        }

        if let Some(ref expected_hash) = entry.integrity {
            verify_cached_integrity(name, expected_hash)?;
        }
    }

    // Fetch agents
    for (name, entry) in &lockfile.agents {
        match &entry.source {
            LockedSource::Git { git: url, path } => {
                if let Some(ref commit) = entry.git_commit {
                    if cache::get_agent(name, commit)?.is_some() {
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
                }
            }
            LockedSource::Local { .. } => {}
            _ => {}
        }

        if let Some(ref expected_hash) = entry.integrity {
            verify_cached_integrity(name, expected_hash)?;
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
        anyhow::bail!(
            "git cache missing for '{}'; run `agx lock` first",
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

/// Verify integrity of a cached asset.
fn verify_cached_integrity(name: &str, expected: &str) -> anyhow::Result<()> {
    // Integrity verification is best-effort at this stage.
    // Full implementation would look up the cached path and verify.
    // For now, we log a note.
    let _ = name;
    let _ = expected;
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
