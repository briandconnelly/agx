//! `agx outdated` — list assets with newer versions available.

use clap::Args;
use console::style;
use std::collections::BTreeMap;

use agx::lockfile::types::{LockedSource, Lockfile};
use agx::manifest::types::AgentEnvironment;
use agx::resolver::semver_utils;
use agx::sources::registry;

#[derive(Debug, Args)]
pub struct OutdatedArgs {}

/// A single row in the outdated table.
struct OutdatedEntry {
    /// Package name.
    name: String,
    /// Currently locked version.
    current: String,
    /// Best version that still satisfies the manifest constraint.
    wanted: String,
    /// Absolute latest version in the registry.
    latest: String,
}

/// Run the `agx outdated` command.
///
/// For each registry plugin in the lockfile, checks whether a newer version
/// exists within the manifest constraint ("wanted") or at all ("latest").
/// Prints a table summarizing the results.
pub fn run(_args: OutdatedArgs) -> anyhow::Result<()> {
    let ctx = super::ProjectContext::load()?;
    let manifest = &ctx.manifest;

    let lockfile = match ctx.lockfile {
        Some(lf) => lf,
        None => {
            eprintln!(
                "no lockfile found; run {} first",
                style("agx install").cyan()
            );
            return Ok(());
        }
    };

    // Fetch registry indexes.
    let indexes = fetch_registry_indexes(manifest)?;

    // Check each registry plugin for updates.
    let entries = find_outdated(manifest, &lockfile, &indexes);

    if entries.is_empty() {
        eprintln!("{}", style("all packages are up to date").green());
        return Ok(());
    }

    print_table(&entries);
    Ok(())
}

/// Fetch cached registry indexes. If an index is not cached, prints a
/// suggestion to run `agx install` first.
fn fetch_registry_indexes(
    manifest: &AgentEnvironment,
) -> anyhow::Result<BTreeMap<String, registry::RegistryIndex>> {
    let mut indexes = BTreeMap::new();

    for (reg_name, reg_config) in &manifest.registries {
        let cache_dir = agx::cache::registry_dir()?.join(reg_name);
        if !cache_dir.exists() {
            eprintln!(
                "{} registry '{}' is not cached; run {} to fetch it",
                style("note:").blue().bold(),
                reg_name,
                style("agx install").cyan(),
            );
            continue;
        }
        match registry::fetch_index(&reg_config.url, &cache_dir) {
            Ok(index) => {
                indexes.insert(reg_name.clone(), index);
            }
            Err(e) => {
                eprintln!(
                    "{} could not read registry '{}': {}",
                    style("warning:").yellow().bold(),
                    reg_name,
                    e
                );
            }
        }
    }

    Ok(indexes)
}

/// Find outdated registry plugins by comparing locked versions to available
/// versions in the registry indexes.
fn find_outdated(
    manifest: &AgentEnvironment,
    lockfile: &Lockfile,
    indexes: &BTreeMap<String, registry::RegistryIndex>,
) -> Vec<OutdatedEntry> {
    let mut entries = Vec::new();

    for (name, locked) in &lockfile.plugins {
        // Only check registry-sourced plugins.
        let (reg_name, current_str) = match &locked.source {
            LockedSource::Registry {
                registry,
                resolved_version,
                ..
            } => (registry.as_str(), resolved_version.as_str()),
            _ => continue,
        };

        let current = match semver::Version::parse(current_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Look up the registry index.
        let index = match indexes.get(reg_name) {
            Some(idx) => idx,
            None => continue,
        };

        // Look up available versions for this plugin.
        let available = match registry::lookup_plugin(index, name) {
            Some(v) => v,
            None => continue,
        };

        let all_versions: Vec<semver::Version> =
            available.iter().map(|pv| pv.version.clone()).collect();

        // Determine "wanted": best match within the manifest constraint.
        let constraint_str = manifest
            .plugins
            .get(name)
            .and_then(|dep| dep.version.as_deref())
            .unwrap_or("*");

        let wanted = match semver_utils::parse_constraint(constraint_str) {
            Ok(req) => semver_utils::resolve_best(&all_versions, &req).unwrap_or(current.clone()),
            Err(_) => current.clone(),
        };

        // Determine "latest": absolute newest version.
        let latest = all_versions
            .iter()
            .max()
            .cloned()
            .unwrap_or(current.clone());

        // Only show if there's something newer.
        if wanted > current || latest > current {
            entries.push(OutdatedEntry {
                name: name.clone(),
                current: current.to_string(),
                wanted: wanted.to_string(),
                latest: latest.to_string(),
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Print the outdated table with aligned columns.
fn print_table(entries: &[OutdatedEntry]) {
    // Calculate column widths.
    let name_width = entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let current_width = entries
        .iter()
        .map(|e| e.current.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let wanted_width = entries
        .iter()
        .map(|e| e.wanted.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let latest_width = entries
        .iter()
        .map(|e| e.latest.len())
        .max()
        .unwrap_or(6)
        .max(6);

    // Header.
    println!(
        "{:<name_w$}  {:<cur_w$}  {:<want_w$}  {:<lat_w$}",
        style("Package").bold(),
        style("Current").bold(),
        style("Wanted").bold(),
        style("Latest").bold(),
        name_w = name_width,
        cur_w = current_width,
        want_w = wanted_width,
        lat_w = latest_width,
    );

    // Rows.
    for entry in entries {
        let wanted_styled = if entry.wanted != entry.current {
            style(&entry.wanted).yellow().to_string()
        } else {
            style(&entry.wanted).dim().to_string()
        };
        let latest_styled = if entry.latest != entry.current {
            style(&entry.latest).red().to_string()
        } else {
            style(&entry.latest).dim().to_string()
        };

        println!(
            "{:<name_w$}  {:<cur_w$}  {:<want_w$}  {:<lat_w$}",
            entry.name,
            entry.current,
            wanted_styled,
            latest_styled,
            name_w = name_width,
            cur_w = current_width,
            want_w = wanted_width,
            lat_w = latest_width,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::lockfile::types::{LockedPlugin, LockedSource, Lockfile};
    use agx::sources::registry;

    fn sample_index() -> registry::RegistryIndex {
        registry::parse_index(
            r#"{
            "plugins": {
                "quality-review": {
                    "description": "Code quality",
                    "versions": {
                        "1.0.0": { "commit": "aaa" },
                        "1.2.0": { "commit": "bbb" },
                        "1.5.0": { "commit": "ccc" },
                        "2.1.0": { "commit": "ddd" }
                    }
                },
                "test-runner": {
                    "description": "Test runner",
                    "versions": {
                        "0.5.0": { "commit": "eee" },
                        "0.6.0": { "commit": "fff" }
                    }
                }
            }
        }"#,
        )
        .unwrap()
    }

    fn sample_manifest() -> AgentEnvironment {
        serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code: {}
registries:
  official:
    url: https://example.com
plugins:
  quality-review:
    registry: official
    version: ">=1.0.0, <2.0.0"
  test-runner:
    registry: official
    version: "^0.5.0"
"#,
        )
        .unwrap()
    }

    #[test]
    fn finds_outdated_within_constraint() {
        let manifest = sample_manifest();
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "quality-review".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "1.2.0".to_string(),
                },
                git_commit: Some("bbb".to_string()),
                integrity: None,
            },
        );

        let mut indexes = BTreeMap::new();
        indexes.insert("official".to_string(), sample_index());

        let entries = find_outdated(&manifest, &lockfile, &indexes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "quality-review");
        assert_eq!(entries[0].current, "1.2.0");
        assert_eq!(entries[0].wanted, "1.5.0");
        assert_eq!(entries[0].latest, "2.1.0");
    }

    #[test]
    fn up_to_date_not_shown() {
        let manifest = sample_manifest();
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "test-runner".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "0.6.0".to_string(),
                },
                git_commit: Some("fff".to_string()),
                integrity: None,
            },
        );

        let mut indexes = BTreeMap::new();
        indexes.insert("official".to_string(), sample_index());

        let entries = find_outdated(&manifest, &lockfile, &indexes);
        assert!(entries.is_empty());
    }

    #[test]
    fn git_plugins_skipped() {
        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test
backends:
  claude-code: {}
plugins:
  git-plugin:
    git: https://github.com/someone/plugin.git
    tag: v1.0.0
"#,
        )
        .unwrap();

        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "git-plugin".to_string(),
            LockedPlugin {
                source: LockedSource::Git {
                    git: "https://github.com/someone/plugin.git".to_string(),
                    path: None,
                },
                git_commit: Some("abc123".to_string()),
                integrity: None,
            },
        );

        let indexes = BTreeMap::new();
        let entries = find_outdated(&manifest, &lockfile, &indexes);
        assert!(entries.is_empty());
    }

    #[test]
    fn multiple_outdated_sorted() {
        let manifest = sample_manifest();
        let mut lockfile = Lockfile::new();
        lockfile.plugins.insert(
            "quality-review".to_string(),
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
        lockfile.plugins.insert(
            "test-runner".to_string(),
            LockedPlugin {
                source: LockedSource::Registry {
                    registry: "official".to_string(),
                    path: None,
                    resolved_version: "0.5.0".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );

        let mut indexes = BTreeMap::new();
        indexes.insert("official".to_string(), sample_index());

        let entries = find_outdated(&manifest, &lockfile, &indexes);
        assert_eq!(entries.len(), 2);
        // Sorted alphabetically.
        assert_eq!(entries[0].name, "quality-review");
        assert_eq!(entries[1].name, "test-runner");
    }
}
