//! `agx remove` — remove a dependency from the manifest.

use agx::config;
use agx::manifest;
use anyhow::Context;
use clap::Args;

use super::install;
use super::lock;

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Name of the asset to remove
    pub name: String,
}

/// Run the `agx remove` command.
///
/// Loads the manifest, removes the named dependency from whichever
/// section it's in, saves the manifest, and re-runs lock + install.
pub fn run(args: RemoveArgs) -> anyhow::Result<()> {
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

    let mut env = manifest::load(&manifest_path)?;

    let name = &args.name;
    let mut found = false;

    if env.plugins.remove(name).is_some() {
        eprintln!("removed plugin '{name}'");
        found = true;
    }
    if env.skills.remove(name).is_some() {
        eprintln!("removed skill '{name}'");
        found = true;
    }
    if env.agents.remove(name).is_some() {
        eprintln!("removed agent '{name}'");
        found = true;
    }
    if env.mcp_servers.remove(name).is_some() {
        eprintln!("removed MCP server '{name}'");
        found = true;
    }

    if !found {
        anyhow::bail!(
            "'{name}' not found in plugins, skills, agents, or mcp-servers"
        );
    }

    // Save the updated manifest
    manifest::save(&env, &manifest_path)?;
    eprintln!("manifest updated.");

    // Re-run lock + install
    let lockfile = lock::run_lock(&env, &project_root)?;
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    agx::lockfile::save(&lockfile, &lockfile_path)?;

    install::run(install::InstallArgs {})?;

    Ok(())
}
