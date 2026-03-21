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

/// A resolved plugin pinned to an exact version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPlugin {
    /// Source information (registry name + resolved version, or git URL).
    pub source: LockedSource,

    /// Exact git commit SHA.
    #[serde(default)]
    pub git_commit: Option<String>,

    /// SHA-256 integrity hash of the downloaded content.
    #[serde(default)]
    pub integrity: Option<String>,
}

/// A resolved skill pinned to an exact commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedSkill {
    /// Source information.
    pub source: LockedSource,

    /// Exact git commit SHA.
    #[serde(default)]
    pub git_commit: Option<String>,

    /// SHA-256 integrity hash.
    #[serde(default)]
    pub integrity: Option<String>,
}

/// A resolved agent pinned to an exact commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedAgent {
    /// Source information.
    pub source: LockedSource,

    /// Exact git commit SHA.
    #[serde(default)]
    pub git_commit: Option<String>,

    /// SHA-256 integrity hash.
    #[serde(default)]
    pub integrity: Option<String>,
}

/// Source descriptor for a locked entry.
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

/// Simple ISO 8601 timestamp without pulling in chrono.
fn chrono_now() -> String {
    // For the MVP, we use a placeholder. A proper implementation would use
    // std::time::SystemTime or the chrono crate.
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", duration.as_secs())
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
