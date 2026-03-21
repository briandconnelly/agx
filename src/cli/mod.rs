//! CLI argument parsing and command dispatch.

pub mod add;
pub mod backends_cmd;
pub mod cache_cmd;
pub mod check;
pub mod clean;
pub mod completions;
pub mod diff;
pub mod format_helpers;
pub mod generate;
pub mod graph;
pub mod info;
pub mod init;
pub mod install;
pub mod list;
pub mod lock;
pub mod outdated;
pub mod publish;
pub mod registry_cmd;
pub mod remove;
pub mod run;
pub mod update;
pub mod which_cmd;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

/// Common project context loaded by most CLI commands.
///
/// Encapsulates the boilerplate of finding the manifest, loading it,
/// determining the project root, and optionally loading the lockfile.
pub struct ProjectContext {
    /// The parsed agent-environment manifest.
    pub manifest: agx::manifest::types::AgentEnvironment,
    /// Absolute path to the manifest file.
    pub manifest_path: PathBuf,
    /// Directory containing the manifest (project root).
    pub project_root: PathBuf,
    /// The lockfile, if one exists on disk.
    pub lockfile: Option<agx::lockfile::types::Lockfile>,
}

impl ProjectContext {
    /// Load the project context from the current directory.
    ///
    /// Finds the manifest by walking up from `cwd`, loads it, and optionally
    /// loads the lockfile if it exists alongside the manifest.
    pub fn load() -> anyhow::Result<Self> {
        let cwd = std::env::current_dir().context("getting current directory")?;
        let manifest_path = agx::config::find_manifest(&cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "no {} found in {} or parent directories",
                agx::config::MANIFEST_FILE,
                cwd.display()
            )
        })?;
        let project_root = manifest_path
            .parent()
            .expect("manifest must have a parent directory")
            .to_path_buf();
        let manifest = agx::manifest::load(&manifest_path)?;
        let lockfile_path = project_root.join(agx::config::LOCKFILE_FILE);
        let lockfile = if lockfile_path.exists() {
            Some(agx::lockfile::load(&lockfile_path)?)
        } else {
            None
        };
        Ok(Self {
            manifest,
            manifest_path,
            project_root,
            lockfile,
        })
    }
}

/// agx — Agent Environment Manager
///
/// A package manager for AI agent environments. Manages plugins, skills,
/// subagents, and MCP servers across agent frameworks like Claude Code
/// and Gemini CLI.
#[derive(Debug, Parser)]
#[command(name = "agx", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize a new agx project or import existing configuration
    Init(init::InitArgs),

    /// Resolve, download, and generate backend configurations
    Install(install::InstallArgs),

    /// Show what would change if `agx generate` were run now
    Diff(diff::DiffArgs),

    /// Regenerate backend configs from existing lockfile and cache
    Generate(generate::GenerateArgs),

    /// Resolve and write lockfile without downloading or generating
    Lock(lock::LockArgs),

    /// Add a dependency to the manifest
    Add(add::AddArgs),

    /// Remove a dependency from the manifest
    Remove(remove::RemoveArgs),

    /// Re-resolve dependencies within manifest constraints
    Update(update::UpdateArgs),

    /// Visualize the dependency graph (tree or DOT format)
    Graph(graph::GraphArgs),

    /// Show the current dependency tree
    List(list::ListArgs),

    /// Validate manifest and detect conflicts
    Check(check::CheckArgs),

    /// Remove .agx/ and all generated backend directories
    Clean(clean::CleanArgs),

    /// Launch a backend in the managed environment
    Run(run::RunArgs),

    /// Cache management commands
    Cache(cache_cmd::CacheArgs),

    /// Registry management commands
    Registry(registry_cmd::RegistryArgs),

    /// Show details about an installed asset
    Info(info::InfoArgs),

    /// Show the resolved source and file path of an asset
    Which(which_cmd::WhichArgs),

    /// List assets with newer versions available
    Outdated(outdated::OutdatedArgs),

    /// Publish an asset to a marketplace registry
    Publish(publish::PublishArgs),

    /// List available backends
    Backends(backends_cmd::BackendsArgs),

    /// Generate shell completions
    Completions(completions::CompletionsArgs),
}

/// Dispatch a parsed CLI command to the appropriate handler.
pub fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init(args) => init::run(args),
        Commands::Install(args) => install::run(args),
        Commands::Diff(args) => diff::run(args),
        Commands::Generate(args) => generate::run(args),
        Commands::Lock(args) => lock::run(args),
        Commands::Add(args) => add::run(args),
        Commands::Remove(args) => remove::run(args),
        Commands::Update(args) => update::run(args),
        Commands::Graph(args) => graph::run(args),
        Commands::List(args) => list::run(args),
        Commands::Check(args) => check::run(args),
        Commands::Clean(args) => clean::run(args),
        Commands::Run(args) => run::run(args),
        Commands::Cache(args) => cache_cmd::run(args),
        Commands::Registry(args) => registry_cmd::run(args),
        Commands::Info(args) => info::run(args),
        Commands::Which(args) => which_cmd::run(args),
        Commands::Outdated(args) => outdated::run(args),
        Commands::Publish(args) => publish::run(args),
        Commands::Backends(args) => backends_cmd::run(args),
        Commands::Completions(args) => completions::run(args),
    }
}
