//! `agx update` — re-resolve dependencies within manifest constraints.

use anyhow::{bail, Context};
use clap::Args;
use console::style;
use std::collections::BTreeMap;
use std::path::PathBuf;

use agx::backends::{get_backend, ResolvedEnvironment};
use agx::config;
use agx::lockfile;
use agx::lockfile::types::{LockedEntry, LockedSource, Lockfile};
use agx::manifest::types::AgentEnvironment;
use agx::resolver;

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Name of a specific dependency to update (all if omitted)
    pub name: Option<String>,
}

/// Run the `agx update` command.
///
/// Re-resolves dependencies using fresh registry/git data, writes the updated
/// lockfile, regenerates all backends, and prints a diff of what changed.
pub fn run(args: UpdateArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;

    // Find and load manifest.
    let manifest_path = config::find_manifest(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} found in {} or parent directories",
            config::MANIFEST_FILE,
            cwd.display()
        )
    })?;
    let project_root = manifest_path
        .parent()
        .expect("manifest must have a parent directory");

    let manifest_content =
        std::fs::read_to_string(&manifest_path).context("reading manifest file")?;
    let manifest: AgentEnvironment =
        serde_yaml::from_str(&manifest_content).context("parsing manifest")?;

    // Load existing lockfile (or start fresh).
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    let old_lockfile = if lockfile_path.exists() {
        lockfile::load(&lockfile_path)?
    } else {
        Lockfile::new()
    };

    // Build resolve context from registries and git refs.
    let context = super::lock::build_resolve_context(&manifest)?;

    // Resolve.
    let new_lockfile = if let Some(ref name) = args.name {
        // Validate the name exists in the manifest.
        if !manifest.plugins.contains_key(name)
            && !manifest.skills.contains_key(name)
            && !manifest.agents.contains_key(name)
        {
            bail!(
                "dependency '{}' not found in manifest plugins, skills, or agents",
                name
            );
        }
        eprintln!("updating {}...", style(name).cyan());
        resolver::resolve_single(name, &manifest, &old_lockfile, &context)
            .map_err(|e| anyhow::anyhow!("{}", e))?
    } else {
        eprintln!("updating all dependencies...");
        let plugin_assets = BTreeMap::new(); // TODO: populate from cached plugin manifests
        resolver::resolve_all(&manifest, &context, &plugin_assets)
            .map_err(|e| anyhow::anyhow!("{}", e))?
    };

    // Save new lockfile.
    lockfile::save(&new_lockfile, &lockfile_path)?;
    eprintln!("wrote {}", config::LOCKFILE_FILE);

    // Regenerate all backends.
    let cache_dir = agx::cache::cache_dir()?;
    let resolved = ResolvedEnvironment {
        manifest: &manifest,
        lockfile: &new_lockfile,
        cache_dir: &cache_dir,
    };
    let output_dir = PathBuf::from(project_root);

    for name in manifest.backends.keys() {
        if let Some(backend) = get_backend(name) {
            eprintln!("generating {} backend...", backend.name());
            backend
                .generate(&resolved, &output_dir)
                .with_context(|| format!("generating {} backend", backend.name()))?;
        }
    }

    // Print diff.
    print_diff(&old_lockfile, &new_lockfile);

    Ok(())
}

/// Print a human-readable diff of what changed between two lockfiles.
pub fn print_diff(old: &Lockfile, new: &Lockfile) {
    let mut changes = Vec::new();

    // Check plugins.
    diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
    diff_map("skill", &old.skills, &new.skills, &mut changes);
    diff_map("agent", &old.agents, &new.agents, &mut changes);

    if changes.is_empty() {
        eprintln!("{}", style("no changes").dim());
    } else {
        eprintln!("\n{}", style("Changes:").bold());
        for line in &changes {
            eprintln!("  {}", line);
        }
    }
}

/// Diff two BTreeMaps of lockfile entries and collect human-readable change lines.
fn diff_map<V: HasVersion>(
    kind: &str,
    old: &BTreeMap<String, V>,
    new: &BTreeMap<String, V>,
    changes: &mut Vec<String>,
) {
    // Added entries.
    for name in new.keys() {
        if !old.contains_key(name) {
            changes.push(format!(
                "{} {} {}",
                style("+").green().bold(),
                kind,
                style(name).green(),
            ));
        }
    }

    // Removed entries.
    for name in old.keys() {
        if !new.contains_key(name) {
            changes.push(format!(
                "{} {} {}",
                style("-").red().bold(),
                kind,
                style(name).red(),
            ));
        }
    }

    // Changed entries.
    for (name, old_entry) in old {
        if let Some(new_entry) = new.get(name) {
            let old_ver = old_entry.version_display();
            let new_ver = new_entry.version_display();
            if old_ver != new_ver {
                changes.push(format!(
                    "{} {} {} {} -> {}",
                    style("~").yellow().bold(),
                    kind,
                    style(name).yellow(),
                    style(&old_ver).dim(),
                    style(&new_ver).cyan(),
                ));
            }
        }
    }
}

/// Trait for extracting a display version from lockfile entry types.
trait HasVersion {
    fn version_display(&self) -> String;
}

impl HasVersion for LockedEntry {
    fn version_display(&self) -> String {
        match &self.source {
            LockedSource::Registry {
                resolved_version, ..
            } => resolved_version.clone(),
            LockedSource::Git { git, .. } => self
                .git_commit
                .as_deref()
                .map(|c| format!("{}#{}", git, &c[..7.min(c.len())]))
                .unwrap_or_else(|| git.clone()),
            LockedSource::Npm {
                package,
                resolved_version,
                ..
            } => resolved_version
                .as_deref()
                .map_or_else(|| package.clone(), |v| format!("{package}@{v}")),
            LockedSource::Local { local } => local.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::lockfile::types::*;

    #[test]
    fn diff_detects_added_plugin() {
        let old = Lockfile::new();
        let mut new = Lockfile::new();
        new.plugins.insert(
            "new-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );

        let mut changes = Vec::new();
        diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
        assert_eq!(changes.len(), 1);
        // The change line should mention "new-plugin" (strip ANSI for checking).
        let plain = console::strip_ansi_codes(&changes[0]);
        assert!(plain.contains("new-plugin"));
        assert!(plain.contains("+"));
    }

    #[test]
    fn diff_detects_removed_plugin() {
        let mut old = Lockfile::new();
        old.plugins.insert(
            "removed-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        let new = Lockfile::new();

        let mut changes = Vec::new();
        diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
        assert_eq!(changes.len(), 1);
        let plain = console::strip_ansi_codes(&changes[0]);
        assert!(plain.contains("removed-plugin"));
        assert!(plain.contains("-"));
    }

    #[test]
    fn diff_detects_version_change() {
        let mut old = Lockfile::new();
        old.plugins.insert(
            "my-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        let mut new = Lockfile::new();
        new.plugins.insert(
            "my-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.5.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );

        let mut changes = Vec::new();
        diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
        assert_eq!(changes.len(), 1);
        let plain = console::strip_ansi_codes(&changes[0]);
        assert!(plain.contains("my-plugin"));
        assert!(plain.contains("1.0.0"));
        assert!(plain.contains("1.5.0"));
        assert!(plain.contains("~"));
    }

    #[test]
    fn diff_no_changes() {
        let mut old = Lockfile::new();
        old.plugins.insert(
            "stable".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        let new = old.clone();

        let mut changes = Vec::new();
        diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
        assert!(changes.is_empty());
    }

    #[test]
    fn diff_mixed_changes() {
        let mut old = Lockfile::new();
        old.plugins.insert(
            "kept".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "r".to_string(),
                    path: None,
                    resolved_version: "1.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        old.plugins.insert(
            "removed".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "r".to_string(),
                    path: None,
                    resolved_version: "2.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );

        let mut new = Lockfile::new();
        new.plugins.insert(
            "kept".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "r".to_string(),
                    path: None,
                    resolved_version: "1.2.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        new.plugins.insert(
            "added".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "r".to_string(),
                    path: None,
                    resolved_version: "3.0.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );

        let mut changes = Vec::new();
        diff_map("plugin", &old.plugins, &new.plugins, &mut changes);
        // Should have: +added, -removed, ~kept (version change)
        assert_eq!(changes.len(), 3);
    }
}
