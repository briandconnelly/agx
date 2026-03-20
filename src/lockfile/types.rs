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
    /// From a marketplace registry.
    Registry {
        registry: String,
        resolved_version: String,
    },
    /// From a git repository.
    Git {
        git: String,
        #[serde(default)]
        path: Option<String>,
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
}
