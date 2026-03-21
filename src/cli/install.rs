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

#[cfg(test)]
mod tests {
    use super::*;
    use agx::cache::integrity::hash_dir;
    use agx::lockfile::types::{LockedEntry, LockedSource, Lockfile};
    use agx::manifest::types::AgentEnvironment;
    use std::fs;

    /// Helper: set AGX_CACHE_DIR to a temp directory for test isolation.
    fn with_temp_cache(f: impl FnOnce(&Path)) {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };
        f(tmp.path());
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    // -----------------------------------------------------------------------
    // verify_cached_integrity
    // -----------------------------------------------------------------------

    #[test]
    fn verify_cached_integrity_success() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"hello").unwrap();
        fs::write(dir.path().join("other.txt"), b"world").unwrap();

        let expected = hash_dir(dir.path()).unwrap();
        let result = verify_cached_integrity("test-asset", dir.path(), &expected);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_cached_integrity_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"hello").unwrap();

        let wrong_hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let result = verify_cached_integrity("test-asset", dir.path(), wrong_hash);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("integrity check failed"));
        assert!(err_msg.contains("test-asset"));
    }

    #[test]
    fn verify_cached_integrity_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let expected = hash_dir(dir.path()).unwrap();
        let result = verify_cached_integrity("empty", dir.path(), &expected);
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // load_or_lock
    // -----------------------------------------------------------------------

    #[test]
    fn load_or_lock_no_lockfile_creates_one() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-project
backends:
  claude-code: {}
skills:
  my-skill:
    path: ./skills/my-skill
"#,
            )
            .unwrap();

            let project = tempfile::tempdir().unwrap();
            let lockfile_path = project.path().join("agent-environment.lock");

            assert!(!lockfile_path.exists());

            let lockfile = load_or_lock(&manifest, project.path(), &lockfile_path).unwrap();

            assert!(lockfile_path.exists());
            assert_eq!(lockfile.skills.len(), 1);
            assert!(lockfile.skills.contains_key("my-skill"));
        });
    }

    #[test]
    fn load_or_lock_fresh_lockfile_returns_it() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-project
backends:
  claude-code: {}
skills:
  my-skill:
    path: ./skills/my-skill
"#,
            )
            .unwrap();

            let project = tempfile::tempdir().unwrap();
            let lockfile_path = project.path().join("agent-environment.lock");

            let lockfile1 = load_or_lock(&manifest, project.path(), &lockfile_path).unwrap();
            assert!(lockfile_path.exists());

            let lockfile2 = load_or_lock(&manifest, project.path(), &lockfile_path).unwrap();
            assert_eq!(lockfile2.skills.len(), lockfile1.skills.len());
            assert!(lockfile2.skills.contains_key("my-skill"));
        });
    }

    #[test]
    fn load_or_lock_stale_lockfile_re_resolves() {
        with_temp_cache(|_cache| {
            let manifest1: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-project
backends:
  claude-code: {}
skills:
  skill-a:
    path: ./skills/a
"#,
            )
            .unwrap();

            let project = tempfile::tempdir().unwrap();
            let lockfile_path = project.path().join("agent-environment.lock");

            let lockfile1 = load_or_lock(&manifest1, project.path(), &lockfile_path).unwrap();
            assert_eq!(lockfile1.skills.len(), 1);

            let manifest2: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-project
backends:
  claude-code: {}
skills:
  skill-a:
    path: ./skills/a
  skill-b:
    path: ./skills/b
"#,
            )
            .unwrap();

            let lockfile2 = load_or_lock(&manifest2, project.path(), &lockfile_path).unwrap();
            assert_eq!(lockfile2.skills.len(), 2);
            assert!(lockfile2.skills.contains_key("skill-a"));
            assert!(lockfile2.skills.contains_key("skill-b"));
        });
    }

    // -----------------------------------------------------------------------
    // extract_from_git_cache
    // -----------------------------------------------------------------------

    #[test]
    fn extract_from_git_cache_repo_not_found() {
        let git_cache = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let store_fn = |_src: &Path| -> anyhow::Result<PathBuf> { Ok(dest.path().to_path_buf()) };

        let result = extract_from_git_cache(
            git_cache.path(),
            "https://github.com/example/nonexistent.git",
            "abc123",
            None,
            &store_fn,
        );
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("git cache missing") || err_msg.contains("re-fetch"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn extract_from_git_cache_with_bare_repo() {
        let git_cache = tempfile::tempdir().unwrap();
        let url = "https://github.com/test/repo.git";
        let slug = agx::sources::git::url_to_slug(url);
        let repo_path = git_cache.path().join(&slug);

        // Init a bare repo with one commit
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .arg(&repo_path)
            .output()
            .unwrap();

        // Create a temporary working repo, add a file, commit, push to bare repo
        let work = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(work.path())
            .output()
            .unwrap();
        fs::write(work.path().join("hello.txt"), "hello world").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(work.path())
            .output()
            .unwrap();
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(work.path())
            .output()
            .unwrap();
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        std::process::Command::new("git")
            .args(["push", repo_path.to_str().unwrap(), "HEAD:refs/heads/main"])
            .current_dir(work.path())
            .output()
            .unwrap();

        // Test extraction
        let store_fn = |src: &Path| -> anyhow::Result<PathBuf> {
            assert!(src.exists());
            Ok(src.to_path_buf())
        };

        let result = extract_from_git_cache(
            git_cache.path(),
            url,
            &commit,
            None,
            &store_fn,
        );
        assert!(result.is_ok(), "extract failed: {:?}", result.err());
    }

    // -----------------------------------------------------------------------
    // fetch_assets — local-only and npm bail
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_local_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "local-plugin".into(),
                LockedEntry {
                    source: LockedSource::Local {
                        local: "./plugins/local".into(),
                    },
                    git_commit: None,
                    integrity: None,
                },
            );
            lockfile.skills.insert(
                "local-skill".into(),
                LockedEntry {
                    source: LockedSource::Local {
                        local: "./skills/local".into(),
                    },
                    git_commit: None,
                    integrity: None,
                },
            );
            lockfile.agents.insert(
                "local-agent".into(),
                LockedEntry {
                    source: LockedSource::Local {
                        local: "./agents/local.md".into(),
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn fetch_assets_npm_bails() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "npm-plugin".into(),
                LockedEntry {
                    source: LockedSource::Npm {
                        package: "@scope/my-plugin".into(),
                        resolved_version: Some("1.0.0".into()),
                        npm_registry: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("npm source"));
            assert!(err_msg.contains("not yet supported"));
        });
    }

    #[test]
    fn fetch_assets_empty_lockfile() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let lockfile = Lockfile::new();
            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // run_generate
    // -----------------------------------------------------------------------

    #[test]
    fn run_generate_empty_lockfile() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-generate
backends:
  claude-code: {}
"#,
            )
            .unwrap();

            let lockfile = Lockfile::new();
            let project = tempfile::tempdir().unwrap();

            let result = run_generate(&manifest, &lockfile, project.path());
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_generate_unknown_backend_errors() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-generate
backends:
  nonexistent-backend: {}
"#,
            )
            .unwrap();

            let lockfile = Lockfile::new();
            let project = tempfile::tempdir().unwrap();

            let result = run_generate(&manifest, &lockfile, project.path());
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(err_msg.contains("no built-in backend"));
        });
    }

    #[test]
    fn run_generate_multiple_backends() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-multi
backends:
  claude-code: {}
  gemini-cli: {}
"#,
            )
            .unwrap();

            let lockfile = Lockfile::new();
            let project = tempfile::tempdir().unwrap();

            let result = run_generate(&manifest, &lockfile, project.path());
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // extract_from_git_cache — offline mode
    // -----------------------------------------------------------------------

    #[test]
    fn extract_from_git_cache_offline_mode() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let git_cache = tempfile::tempdir().unwrap();

        // Set offline mode
        unsafe { std::env::set_var("AGX_OFFLINE", "1") };

        let store_fn = |_src: &Path| -> anyhow::Result<PathBuf> { Ok(PathBuf::from("/tmp")) };

        let result = extract_from_git_cache(
            git_cache.path(),
            "https://github.com/example/repo.git",
            "abc123",
            None,
            &store_fn,
        );
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("AGX_OFFLINE") || err_msg.contains("git cache missing"),
            "unexpected error: {err_msg}"
        );

        unsafe { std::env::remove_var("AGX_OFFLINE") };
    }

    // -----------------------------------------------------------------------
    // extract_from_git_cache — with subpath
    // -----------------------------------------------------------------------

    #[test]
    fn extract_from_git_cache_with_subpath() {
        let git_cache = tempfile::tempdir().unwrap();
        let url = "https://github.com/test/subpath-repo.git";
        let slug = agx::sources::git::url_to_slug(url);
        let repo_path = git_cache.path().join(&slug);

        // Init bare repo
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .arg(&repo_path)
            .output()
            .unwrap();

        // Create working repo with a subdirectory
        let work = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(work.path())
            .output()
            .unwrap();

        // Create a subdirectory with a file
        fs::create_dir_all(work.path().join("plugins/my-plugin")).unwrap();
        fs::write(work.path().join("plugins/my-plugin/plugin.yml"), "name: my-plugin").unwrap();
        fs::write(work.path().join("root-file.txt"), "root content").unwrap();

        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial with subdir"])
            .current_dir(work.path())
            .output()
            .unwrap();
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(work.path())
            .output()
            .unwrap();
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        std::process::Command::new("git")
            .args(["push", repo_path.to_str().unwrap(), "HEAD:refs/heads/main"])
            .current_dir(work.path())
            .output()
            .unwrap();

        // Extract with a subpath
        let store_fn = |src: &Path| -> anyhow::Result<PathBuf> {
            assert!(src.exists(), "source dir should exist");
            Ok(src.to_path_buf())
        };

        let result = extract_from_git_cache(
            git_cache.path(),
            url,
            &commit,
            Some("plugins/my-plugin"),
            &store_fn,
        );
        assert!(result.is_ok(), "extract with subpath failed: {:?}", result.err());
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git plugin with commit
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_plugin_with_commit() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            // Create a bare repo in the git cache
            let url = "https://github.com/test/plugin-repo.git";
            let slug = agx::sources::git::url_to_slug(url);
            let git_cache = cache.join("git");
            fs::create_dir_all(&git_cache).unwrap();
            let repo_path = git_cache.join(&slug);

            std::process::Command::new("git")
                .args(["init", "--bare"])
                .arg(&repo_path)
                .output()
                .unwrap();

            let work = tempfile::tempdir().unwrap();
            std::process::Command::new("git")
                .args(["init"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(work.path())
                .output()
                .unwrap();
            fs::write(work.path().join("plugin.yml"), "name: my-plugin").unwrap();
            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["commit", "-m", "initial"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            std::process::Command::new("git")
                .args(["push", repo_path.to_str().unwrap(), "HEAD:refs/heads/main"])
                .current_dir(work.path())
                .output()
                .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "my-git-plugin".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: url.to_string(),
                        path: None,
                    },
                    git_commit: Some(commit.clone()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok(), "fetch_assets for git plugin failed: {:?}", result.err());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git skill with commit
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_skill_with_commit() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let url = "https://github.com/test/skill-repo.git";
            let slug = agx::sources::git::url_to_slug(url);
            let git_cache = cache.join("git");
            fs::create_dir_all(&git_cache).unwrap();
            let repo_path = git_cache.join(&slug);

            std::process::Command::new("git")
                .args(["init", "--bare"])
                .arg(&repo_path)
                .output()
                .unwrap();

            let work = tempfile::tempdir().unwrap();
            std::process::Command::new("git")
                .args(["init"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(work.path())
                .output()
                .unwrap();
            fs::write(work.path().join("skill.yml"), "name: my-skill").unwrap();
            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["commit", "-m", "initial"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            std::process::Command::new("git")
                .args(["push", repo_path.to_str().unwrap(), "HEAD:refs/heads/main"])
                .current_dir(work.path())
                .output()
                .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.skills.insert(
                "my-git-skill".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: url.to_string(),
                        path: None,
                    },
                    git_commit: Some(commit.clone()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok(), "fetch_assets for git skill failed: {:?}", result.err());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git agent with commit
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_agent_with_commit() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let url = "https://github.com/test/agent-repo.git";
            let slug = agx::sources::git::url_to_slug(url);
            let git_cache = cache.join("git");
            fs::create_dir_all(&git_cache).unwrap();
            let repo_path = git_cache.join(&slug);

            std::process::Command::new("git")
                .args(["init", "--bare"])
                .arg(&repo_path)
                .output()
                .unwrap();

            let work = tempfile::tempdir().unwrap();
            std::process::Command::new("git")
                .args(["init"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.email", "test@test.com"])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(work.path())
                .output()
                .unwrap();
            fs::write(work.path().join("agent.md"), "# My Agent").unwrap();
            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(work.path())
                .output()
                .unwrap();
            std::process::Command::new("git")
                .args(["commit", "-m", "initial"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(work.path())
                .output()
                .unwrap();
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            std::process::Command::new("git")
                .args(["push", repo_path.to_str().unwrap(), "HEAD:refs/heads/main"])
                .current_dir(work.path())
                .output()
                .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.agents.insert(
                "my-git-agent".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: url.to_string(),
                        path: Some("agent.md".to_string()),
                    },
                    git_commit: Some(commit.clone()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok(), "fetch_assets for git agent failed: {:?}", result.err());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git plugin already cached
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_plugin_already_cached() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            // Pre-populate the cache so it skips fetch
            let commit = "abcdef1234567890abcdef1234567890abcdef12";
            let plugin_dir = cache.join("plugins").join("cached-plugin").join(commit);
            fs::create_dir_all(&plugin_dir).unwrap();
            fs::write(plugin_dir.join("plugin.yml"), "name: cached").unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "cached-plugin".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/cached.git".to_string(),
                        path: None,
                    },
                    git_commit: Some(commit.to_string()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git skill already cached
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_skill_already_cached() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let commit = "abcdef1234567890abcdef1234567890abcdef12";
            let skill_dir = cache.join("skills").join("cached-skill").join(commit);
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(skill_dir.join("skill.yml"), "name: cached").unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.skills.insert(
                "cached-skill".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/cached-skill.git".to_string(),
                        path: None,
                    },
                    git_commit: Some(commit.to_string()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git agent already cached
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_agent_already_cached() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let commit = "abcdef1234567890abcdef1234567890abcdef12";
            let agent_dir = cache.join("agents").join("cached-agent").join(commit);
            fs::create_dir_all(&agent_dir).unwrap();
            fs::write(agent_dir.join("agent.md"), "# Cached Agent").unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.agents.insert(
                "cached-agent".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/cached-agent.git".to_string(),
                        path: None,
                    },
                    git_commit: Some(commit.to_string()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — registry plugin without git_commit (no-op)
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_registry_plugin_no_commit() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "reg-plugin".into(),
                LockedEntry {
                    source: LockedSource::Registry {
                        registry: "default".into(),
                        resolved_version: "1.0.0".into(),
                        path: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            // Without a git_commit, the registry path should be skipped (no fetch attempt)
            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — registry plugin already cached
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_registry_plugin_already_cached() {
        with_temp_cache(|cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            // Pre-populate the cache for a registry plugin
            let plugin_dir = cache.join("plugins").join("cached-reg-plugin").join("1.0.0");
            fs::create_dir_all(&plugin_dir).unwrap();
            fs::write(plugin_dir.join("plugin.yml"), "name: cached-reg").unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "cached-reg-plugin".into(),
                LockedEntry {
                    source: LockedSource::Registry {
                        registry: "default".into(),
                        resolved_version: "1.0.0".into(),
                        path: None,
                    },
                    git_commit: Some("abc123".into()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — registry plugin with missing registry in manifest
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_registry_plugin_missing_registry_errors() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "reg-plugin".into(),
                LockedEntry {
                    source: LockedSource::Registry {
                        registry: "nonexistent-registry".into(),
                        resolved_version: "1.0.0".into(),
                        path: None,
                    },
                    git_commit: Some("abc123".into()),
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_err());
            let err_msg = format!("{}", result.unwrap_err());
            assert!(
                err_msg.contains("nonexistent-registry") && err_msg.contains("not found in manifest"),
                "unexpected error: {err_msg}"
            );
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git plugin without commit (no-op)
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_plugin_no_commit_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.plugins.insert(
                "no-commit-plugin".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/repo.git".to_string(),
                        path: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git skill without commit (no-op)
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_skill_no_commit_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.skills.insert(
                "no-commit-skill".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/repo.git".to_string(),
                        path: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — git agent without commit (no-op)
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_git_agent_no_commit_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.agents.insert(
                "no-commit-agent".into(),
                LockedEntry {
                    source: LockedSource::Git {
                        git: "https://github.com/test/repo.git".to_string(),
                        path: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — skill with non-git source is no-op
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_skill_npm_source_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.skills.insert(
                "npm-skill".into(),
                LockedEntry {
                    source: LockedSource::Npm {
                        package: "@scope/my-skill".into(),
                        resolved_version: Some("1.0.0".into()),
                        npm_registry: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            // Skills with non-git/non-local sources fall through the _ => {} arm
            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // fetch_assets — agent with non-git source is no-op
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_assets_agent_npm_source_is_noop() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                "name: test\nbackends:\n  claude-code: {}\n",
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.agents.insert(
                "npm-agent".into(),
                LockedEntry {
                    source: LockedSource::Npm {
                        package: "@scope/my-agent".into(),
                        resolved_version: Some("1.0.0".into()),
                        npm_registry: None,
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let result = fetch_assets(&manifest, &lockfile);
            assert!(result.is_ok());
        });
    }

    // -----------------------------------------------------------------------
    // verify_cached_integrity with subdirectories
    // -----------------------------------------------------------------------

    #[test]
    fn verify_cached_integrity_with_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub/nested")).unwrap();
        fs::write(dir.path().join("root.txt"), b"root").unwrap();
        fs::write(dir.path().join("sub/mid.txt"), b"mid").unwrap();
        fs::write(dir.path().join("sub/nested/deep.txt"), b"deep").unwrap();

        let expected = hash_dir(dir.path()).unwrap();
        let result = verify_cached_integrity("nested-asset", dir.path(), &expected);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_cached_integrity_tampered_content() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"original").unwrap();
        let expected = hash_dir(dir.path()).unwrap();

        // Tamper with the file
        fs::write(dir.path().join("file.txt"), b"tampered").unwrap();

        let result = verify_cached_integrity("tampered-asset", dir.path(), &expected);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("integrity check failed"));
        assert!(err_msg.contains("tampered-asset"));
    }

    // -----------------------------------------------------------------------
    // run_install_at — full integration
    // -----------------------------------------------------------------------

    #[test]
    fn run_install_at_minimal_project() {
        with_temp_cache(|_cache| {
            let project = tempfile::tempdir().unwrap();
            let manifest_content = r#"
name: minimal-install-test
backends:
  claude-code: {}
"#;
            let manifest_path = project.path().join("agent-environment.yml");
            fs::write(&manifest_path, manifest_content).unwrap();

            let result = run_install_at(&manifest_path);
            assert!(result.is_ok(), "run_install_at failed: {:?}", result.err());

            // Should have created a lockfile
            let lockfile_path = project.path().join("agent-environment.lock");
            assert!(lockfile_path.exists(), "lockfile should be created");
        });
    }

    #[test]
    fn run_install_at_with_local_skill() {
        with_temp_cache(|_cache| {
            let project = tempfile::tempdir().unwrap();
            let manifest_content = r#"
name: local-skill-test
backends:
  claude-code: {}
skills:
  my-local-skill:
    path: ./skills/my-local-skill
"#;
            let manifest_path = project.path().join("agent-environment.yml");
            fs::write(&manifest_path, manifest_content).unwrap();

            // Create the local skill directory so the backend can copy it
            let skill_dir = project.path().join("skills/my-local-skill");
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(skill_dir.join("SKILL.md"), "# My Local Skill").unwrap();

            let result = run_install_at(&manifest_path);
            assert!(result.is_ok(), "run_install_at failed: {:?}", result.err());
        });
    }

    #[test]
    fn run_install_at_with_context_config() {
        with_temp_cache(|_cache| {
            let project = tempfile::tempdir().unwrap();
            let manifest_content = r#"
name: context-test
backends:
  claude-code: {}
context:
  base: "./CONTEXT.md"
  overlays:
    claude-code: "./CONTEXT.claude-code.md"
"#;
            let manifest_path = project.path().join("agent-environment.yml");
            fs::write(&manifest_path, manifest_content).unwrap();

            // Create the context files
            fs::write(project.path().join("CONTEXT.md"), "# Base context").unwrap();
            fs::write(project.path().join("CONTEXT.claude-code.md"), "# Claude overlay").unwrap();

            let result = run_install_at(&manifest_path);
            assert!(result.is_ok(), "run_install_at failed: {:?}", result.err());
        });
    }

    // -----------------------------------------------------------------------
    // run_generate — with local plugins/skills/agents in lockfile
    // -----------------------------------------------------------------------

    #[test]
    fn run_generate_with_local_assets() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-local-assets
backends:
  claude-code: {}
skills:
  local-skill:
    path: ./skills/local-skill
agents:
  local-agent:
    path: ./agents/local-agent.md
"#,
            )
            .unwrap();

            let mut lockfile = Lockfile::new();
            lockfile.skills.insert(
                "local-skill".into(),
                LockedEntry {
                    source: LockedSource::Local {
                        local: "./skills/local-skill".into(),
                    },
                    git_commit: None,
                    integrity: None,
                },
            );
            lockfile.agents.insert(
                "local-agent".into(),
                LockedEntry {
                    source: LockedSource::Local {
                        local: "./agents/local-agent.md".into(),
                    },
                    git_commit: None,
                    integrity: None,
                },
            );

            let project = tempfile::tempdir().unwrap();

            // Create the local skill directory and agent file so the backend can copy them
            let skill_dir = project.path().join("skills/local-skill");
            fs::create_dir_all(&skill_dir).unwrap();
            fs::write(skill_dir.join("SKILL.md"), "# Local Skill").unwrap();

            let agents_dir = project.path().join("agents");
            fs::create_dir_all(&agents_dir).unwrap();
            fs::write(agents_dir.join("local-agent.md"), "# Local Agent").unwrap();

            let result = run_generate(&manifest, &lockfile, project.path());
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_generate_gemini_only() {
        with_temp_cache(|_cache| {
            let manifest: AgentEnvironment = serde_yaml::from_str(
                r#"
name: test-gemini
backends:
  gemini-cli: {}
"#,
            )
            .unwrap();

            let lockfile = Lockfile::new();
            let project = tempfile::tempdir().unwrap();

            let result = run_generate(&manifest, &lockfile, project.path());
            assert!(result.is_ok());
        });
    }
}
