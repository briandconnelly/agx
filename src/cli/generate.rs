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
    let manifest = &ctx.manifest;
    let project_root = &ctx.project_root;
    let lockfile = ctx.lockfile.unwrap_or_else(Lockfile::new);

    // Determine cache directory.
    let cache_dir = agx::cache::cache_dir()?;

    let resolved = ResolvedEnvironment {
        manifest,
        lockfile: &lockfile,
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
