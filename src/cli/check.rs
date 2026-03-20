//! `agx check` — validate manifest, detect conflicts and incompatibilities.

use anyhow::Context;
use clap::Args;
use console::style;

use agx::config;
use agx::manifest;

#[derive(Debug, Args)]
pub struct CheckArgs {}

/// Run the `agx check` command.
pub fn run(_args: CheckArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;

    println!(
        "Checking {}...",
        style(manifest_path.display()).dim()
    );

    let env = manifest::load(&manifest_path)?;

    match manifest::validate::validate(&env) {
        Ok(()) => {
            println!(
                "{} Manifest is valid.",
                style("✓").green().bold()
            );
            Ok(())
        }
        Err(errors) => {
            for err in &errors {
                println!(
                    "  {} {}",
                    style("✗").red().bold(),
                    style(err).red()
                );
            }
            println!();
            println!(
                "{} Found {} error{}.",
                style("✗").red().bold(),
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            );
            std::process::exit(1);
        }
    }
}
