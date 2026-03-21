//! Data types for the `agent-environment.lock` lockfile.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level lockfile structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    /// Lockfile format version.
    pub lockfile_version: u32,

    /// Timestamp when the lockfile was generated.
    pub generated_at: String,

    /// Version of agx that generated this lockfile.
    pub agx_version: String,

    /// Resolved plugin entries.
    #[serde(default)]
    pub plugins: BTreeMap<String, LockedPlugin>,

    /// Resolved skill entries.
    #[serde(default)]
    pub skills: BTreeMap<String, LockedSkill>,

    /// Resolved agent entries.
    #[serde(default)]
    pub agents: BTreeMap<String, LockedAgent>,
}

/// A resolved dependency entry pinned to an exact version or commit.
///
/// Plugins, skills, and agents share the same lockfile structure: a source
/// descriptor, an optional git commit SHA, and an optional integrity hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedEntry {
    /// Source information (registry name + resolved version, git URL, npm package, or local path).
    pub source: LockedSource,

    /// Exact git commit SHA.
    #[serde(default)]
    pub git_commit: Option<String>,

    /// SHA-256 integrity hash of the downloaded content.
    #[serde(default)]
    pub integrity: Option<String>,
}

/// Type alias for backward compatibility — a locked plugin entry.
pub type LockedPlugin = LockedEntry;

/// Type alias for backward compatibility — a locked skill entry.
pub type LockedSkill = LockedEntry;

/// Type alias for backward compatibility — a locked agent entry.
pub type LockedAgent = LockedEntry;

/// Source descriptor for a locked entry.
///
/// **Note:** This enum uses `#[serde(untagged)]`, which means variant order
/// matters during deserialization. Serde tries each variant in declaration
/// order and uses the first one that successfully deserializes. Place more
/// specific variants (with required distinguishing fields) before less
/// specific ones to avoid incorrect matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LockedSource {
    /// From a marketplace registry (relative-path plugin within the registry repo).
    Registry {
        registry: String,
        resolved_version: String,
        #[serde(default)]
        path: Option<String>,
    },
    /// From a git repository (direct git dep or registry plugin with external git source).
    Git {
        git: String,
        #[serde(default)]
        path: Option<String>,
    },
    /// From an npm package.
    Npm {
        package: String,
        #[serde(default)]
        resolved_version: Option<String>,
        #[serde(default)]
        npm_registry: Option<String>,
    },
    /// From a local path.
    Local { local: String },
}

impl Lockfile {
    /// Create a new empty lockfile with current metadata.
    pub fn new() -> Self {
        Self {
            lockfile_version: 1,
            generated_at: chrono_now(),
            agx_version: env!("CARGO_PKG_VERSION").to_string(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
        }
    }
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

/// Produce an ISO 8601 UTC timestamp (e.g. `"2026-03-19T10:30:00Z"`) without
/// pulling in the chrono crate.
fn chrono_now() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Days since epoch
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Convert days to year/month/day.
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lockfile() {
        let yaml = r#"
lockfile_version: 1
generated_at: "2026-03-19T10:30:00Z"
agx_version: "0.1.0"

plugins:
  quality-review:
    source:
      registry: official
      resolved_version: "1.2.0"
    git_commit: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    integrity: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"

skills:
  deploy-checklist:
    source:
      local: ./skills/deploy-checklist

agents:
  my-reviewer:
    source:
      local: ./agents/my-reviewer.md
"#;
        let lockfile: Lockfile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(lockfile.lockfile_version, 1);
        assert_eq!(lockfile.plugins.len(), 1);
        assert_eq!(lockfile.skills.len(), 1);
        assert_eq!(lockfile.agents.len(), 1);
    }

    #[test]
    fn new_lockfile_has_version() {
        let lf = Lockfile::new();
        assert_eq!(lf.lockfile_version, 1);
        assert_eq!(lf.agx_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn npm_source_roundtrip() {
        let yaml = r#"
lockfile_version: 1
generated_at: "2026-03-20T00:00:00Z"
agx_version: "0.1.0"

plugins:
  npm-plugin:
    source:
      package: "@scope/my-plugin"
      resolved_version: "2.3.1"
      npm_registry: "https://registry.npmjs.org"
    integrity: "sha256:abcdef1234567890"
"#;
        let lockfile: Lockfile = serde_yaml::from_str(yaml).unwrap();
        let plugin = &lockfile.plugins["npm-plugin"];
        match &plugin.source {
            LockedSource::Npm {
                package,
                resolved_version,
                npm_registry,
            } => {
                assert_eq!(package, "@scope/my-plugin");
                assert_eq!(resolved_version.as_deref(), Some("2.3.1"));
                assert_eq!(
                    npm_registry.as_deref(),
                    Some("https://registry.npmjs.org")
                );
            }
            other => panic!("expected Npm source, got {:?}", other),
        }

        // Roundtrip: serialize and deserialize back
        let serialized = serde_yaml::to_string(&lockfile).unwrap();
        let roundtripped: Lockfile = serde_yaml::from_str(&serialized).unwrap();
        let plugin2 = &roundtripped.plugins["npm-plugin"];
        match &plugin2.source {
            LockedSource::Npm {
                package,
                resolved_version,
                npm_registry,
            } => {
                assert_eq!(package, "@scope/my-plugin");
                assert_eq!(resolved_version.as_deref(), Some("2.3.1"));
                assert_eq!(
                    npm_registry.as_deref(),
                    Some("https://registry.npmjs.org")
                );
            }
            other => panic!("expected Npm source after roundtrip, got {:?}", other),
        }
    }

    #[test]
    fn registry_source_with_path_roundtrip() {
        let yaml = r#"
lockfile_version: 1
generated_at: "2026-03-20T00:00:00Z"
agx_version: "0.1.0"

plugins:
  subdir-plugin:
    source:
      registry: official
      resolved_version: "1.0.0"
      path: plugins/subdir-plugin
    git_commit: "abc123"
"#;
        let lockfile: Lockfile = serde_yaml::from_str(yaml).unwrap();
        let plugin = &lockfile.plugins["subdir-plugin"];
        match &plugin.source {
            LockedSource::Registry {
                resolved_version,
                path,
                ..
            } => {
                assert_eq!(resolved_version, "1.0.0");
                assert_eq!(path.as_deref(), Some("plugins/subdir-plugin"));
            }
            other => panic!("expected Registry source, got {:?}", other),
        }

        let serialized = serde_yaml::to_string(&lockfile).unwrap();
        let roundtripped: Lockfile = serde_yaml::from_str(&serialized).unwrap();
        let plugin2 = &roundtripped.plugins["subdir-plugin"];
        match &plugin2.source {
            LockedSource::Registry {
                resolved_version,
                path,
                ..
            } => {
                assert_eq!(resolved_version, "1.0.0");
                assert_eq!(path.as_deref(), Some("plugins/subdir-plugin"));
            }
            other => panic!("expected Registry source after roundtrip, got {:?}", other),
        }
    }
}
