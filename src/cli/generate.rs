//! `agx generate` — regenerate backend configs from existing lockfile and cache.

use agx::backends::{get_backend, ResolvedEnvironment};
use agx::context;
use agx::hooks;
use agx::lockfile::types::Lockfile;
use anyhow::{bail, Context};
use clap::Args;

/// Arguments for `agx generate`.
#[derive(Debug, Args)]
pub struct GenerateArgs {
    /// Only generate for this backend (default: all declared backends).
    #[arg(short, long)]
    pub backend: Option<String>,
}

/// Run the `agx generate` command.
///
/// Loads the manifest and lockfile, then calls `generate()` on each backend
/// declared in the manifest. Prints a summary of what was generated.
pub fn run(args: GenerateArgs) -> anyhow::Result<()> {
    let ctx = super::ProjectContext::load()?;
    run_with_context(args, &ctx)
}

/// Inner implementation that takes a pre-loaded project context,
/// separated from `run` so it can be tested without relying on the
/// process working directory.
pub fn run_with_context(
    args: GenerateArgs,
    ctx: &super::ProjectContext,
) -> anyhow::Result<()> {
    let manifest = &ctx.manifest;
    let project_root = &ctx.project_root;
    let lockfile_owned;
    let lockfile = match &ctx.lockfile {
        Some(lf) => lf,
        None => {
            lockfile_owned = Lockfile::new();
            &lockfile_owned
        }
    };

    // Determine cache directory.
    let cache_dir = agx::cache::cache_dir()?;

    let resolved = ResolvedEnvironment {
        manifest,
        lockfile,
        cache_dir: &cache_dir,
    };

    // Filter backends if --backend was specified.
    let backend_names: Vec<&String> = if let Some(ref name) = args.backend {
        if !manifest.backends.contains_key(name) {
            bail!(
                "backend '{}' is not declared in the manifest; available: {:?}",
                name,
                manifest.backends.keys().collect::<Vec<_>>()
            );
        }
        vec![&manifest.backends.keys().find(|k| *k == name).unwrap()]
    } else {
        manifest.backends.keys().collect()
    };

    // Run pre-generate hooks
    let hook_cfg = manifest.hooks.as_ref();
    let pre_generate = hook_cfg.map(|h| h.pre_generate.as_slice()).unwrap_or_default();
    hooks::run_hooks("pre-generate", pre_generate)?;

    let output_dir = project_root;

    for name in &backend_names {
        let backend = get_backend(name).ok_or_else(|| {
            anyhow::anyhow!("no built-in backend for '{}'; available: claude-code, gemini-cli, generic", name)
        })?;

        eprintln!("generating {} backend...", backend.name());
        backend
            .generate(&resolved, &output_dir)
            .with_context(|| format!("generating {} backend", backend.name()))?;
        eprintln!("  done.");
    }

    // Generate context files (CLAUDE.md, GEMINI.md, AGENTS.md, etc.)
    context::generate_context_files(manifest, project_root)
        .context("generating context files")?;

    // Run post-generate hooks
    let post_generate = hook_cfg.map(|h| h.post_generate.as_slice()).unwrap_or_default();
    hooks::run_hooks("post-generate", post_generate)?;

    eprintln!(
        "generated configurations for {} backend(s).",
        backend_names.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn with_temp_cache(f: impl FnOnce()) {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };
        f();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    fn write_manifest(dir: &std::path::Path, yaml: &str) {
        std::fs::write(dir.join("agent-environment.yml"), yaml).unwrap();
    }

    #[test]
    fn run_with_valid_project() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                "name: test\nbackends:\n  claude-code:\n    isolation: strict\n",
            );

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            let args = GenerateArgs { backend: None };
            let result = run_with_context(args, &ctx);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_with_specific_backend() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                "name: test\nbackends:\n  claude-code:\n    isolation: strict\n  gemini-cli: {}\n",
            );

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            let args = GenerateArgs {
                backend: Some("claude-code".to_string()),
            };
            let result = run_with_context(args, &ctx);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_with_unknown_backend_filter_errors() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                "name: test\nbackends:\n  claude-code: {}\n",
            );

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            let args = GenerateArgs {
                backend: Some("nonexistent".to_string()),
            };
            let result = run_with_context(args, &ctx);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("nonexistent"),
                "expected error mentioning 'nonexistent', got: {msg}"
            );
            assert!(
                msg.contains("not declared"),
                "expected 'not declared' error, got: {msg}"
            );
        });
    }

    #[test]
    fn run_no_manifest_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = super::super::ProjectContext::load_from(tmp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no agent-environment.yml found"));
    }

    #[test]
    fn run_with_multiple_backends() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                "name: test\nbackends:\n  claude-code: {}\n  gemini-cli: {}\n",
            );

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            let args = GenerateArgs { backend: None };
            let result = run_with_context(args, &ctx);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_with_no_context_section() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                "name: test\nbackends:\n  claude-code: {}\n",
            );

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            assert!(ctx.manifest.context.is_none());
            let args = GenerateArgs { backend: None };
            let result = run_with_context(args, &ctx);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn run_with_mcp_servers_and_skills() {
        with_temp_cache(|| {
            let tmp = tempfile::tempdir().unwrap();
            write_manifest(
                tmp.path(),
                r#"name: test
backends:
  claude-code: {}
skills:
  deploy:
    path: ./skills/deploy
mcp-servers:
  playwright:
    url: https://mcp.playwright.dev/sse
"#,
            );

            // Create the local skill directory so the backend can copy it
            let skill_dir = tmp.path().join("skills/deploy");
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(skill_dir.join("SKILL.md"), "# Deploy skill").unwrap();

            let ctx = super::super::ProjectContext::load_from(tmp.path()).unwrap();
            let args = GenerateArgs { backend: None };
            let result = run_with_context(args, &ctx);
            assert!(result.is_ok());
        });
    }
}
