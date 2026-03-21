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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: set AGX_CACHE_DIR to a temp directory for test isolation.
    fn with_temp_cache(f: impl FnOnce(&std::path::Path)) {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp.path()) };
        f(tmp.path());
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };
    }

    #[test]
    fn cache_dir_returns_path() {
        with_temp_cache(|cache| {
            let args = CacheArgs {
                command: CacheCommands::Dir,
            };
            let result = run(args);
            assert!(result.is_ok());
            // The cache_dir function should return the temp path we set
            let dir = agx::cache::cache_dir().unwrap();
            assert_eq!(dir, cache);
        });
    }

    #[test]
    fn cache_clean_all_removes_dirs() {
        with_temp_cache(|cache| {
            // Create some cache structure
            let plugins = cache.join("plugins").join("my-plugin").join("1.0.0");
            fs::create_dir_all(&plugins).unwrap();
            fs::write(plugins.join("data"), "test").unwrap();

            let skills = cache.join("skills").join("my-skill").join("abc123");
            fs::create_dir_all(&skills).unwrap();
            fs::write(skills.join("data"), "test").unwrap();

            assert!(cache.join("plugins").exists());
            assert!(cache.join("skills").exists());

            let args = CacheArgs {
                command: CacheCommands::Clean { name: None },
            };
            let result = run(args);
            assert!(result.is_ok());

            // Everything should be cleaned
            assert!(!cache.join("plugins").exists());
            assert!(!cache.join("skills").exists());
        });
    }

    #[test]
    fn cache_clean_specific_asset() {
        with_temp_cache(|cache| {
            // Create cache entries for two assets
            let keep = cache.join("plugins").join("keep-me").join("1.0.0");
            fs::create_dir_all(&keep).unwrap();
            fs::write(keep.join("data"), "keep").unwrap();

            let remove = cache.join("plugins").join("remove-me").join("1.0.0");
            fs::create_dir_all(&remove).unwrap();
            fs::write(remove.join("data"), "remove").unwrap();

            let args = CacheArgs {
                command: CacheCommands::Clean {
                    name: Some("remove-me".to_string()),
                },
            };
            let result = run(args);
            assert!(result.is_ok());

            // Only remove-me should be gone
            assert!(!cache.join("plugins").join("remove-me").exists());
            assert!(cache.join("plugins").join("keep-me").exists());
        });
    }

    #[test]
    fn cache_clean_nonexistent_asset_errors() {
        with_temp_cache(|_cache| {
            let args = CacheArgs {
                command: CacheCommands::Clean {
                    name: Some("nonexistent".to_string()),
                },
            };
            let result = run(args);
            assert!(result.is_err());
        });
    }

    #[test]
    fn cache_list_empty() {
        with_temp_cache(|_cache| {
            let args = CacheArgs {
                command: CacheCommands::List,
            };
            let result = run(args);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn cache_list_with_entries() {
        with_temp_cache(|cache| {
            // Create some cache entries
            let plugin = cache.join("plugins").join("my-plugin").join("1.0.0");
            fs::create_dir_all(&plugin).unwrap();
            fs::write(plugin.join("data"), "hello world").unwrap();

            let git = cache.join("git").join("some-repo.git");
            fs::create_dir_all(&git).unwrap();
            fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();

            let args = CacheArgs {
                command: CacheCommands::List,
            };
            let result = run(args);
            assert!(result.is_ok());
        });
    }
}
