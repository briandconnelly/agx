//! `agx cache` — cache management subcommands.

use clap::{Args, Subcommand};
use console::style;

#[derive(Debug, Args)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommands,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommands {
    /// Print the global cache directory path
    Dir,
    /// Remove cached data (all or for a specific asset)
    Clean {
        /// Specific asset name to clean (all if omitted)
        name: Option<String>,
    },
    /// Show cached assets and disk usage
    List,
}

/// Run a cache subcommand.
pub fn run(args: CacheArgs) -> anyhow::Result<()> {
    match args.command {
        CacheCommands::Dir => {
            let dir = agx::cache::cache_dir()?;
            println!("{}", dir.display());
            Ok(())
        }
        CacheCommands::Clean { name } => {
            if let Some(name) = name {
                agx::cache::clean_asset(&name)?;
                println!(
                    "{} Cleaned cached entries for '{name}'.",
                    style("✓").green()
                );
            } else {
                agx::cache::clean_all()?;
                println!("{} Cache cleaned.", style("✓").green());
            }
            Ok(())
        }
        CacheCommands::List => {
            let entries = agx::cache::list_cached()?;
            if entries.is_empty() {
                println!("Cache is empty.");
                return Ok(());
            }

            let mut total_bytes: u64 = 0;
            for entry in &entries {
                total_bytes += entry.size_bytes;
                let size = agx::cache::format_size(entry.size_bytes);
                if entry.version_or_commit.is_empty() {
                    println!(
                        "  {} {} ({})",
                        style(&entry.category).dim(),
                        entry.name,
                        style(size).cyan(),
                    );
                } else {
                    println!(
                        "  {} {}@{} ({})",
                        style(&entry.category).dim(),
                        entry.name,
                        entry.version_or_commit,
                        style(size).cyan(),
                    );
                }
            }
            println!();
            println!(
                "Total: {} entries, {}",
                entries.len(),
                agx::cache::format_size(total_bytes)
            );
            Ok(())
        }
    }
}
