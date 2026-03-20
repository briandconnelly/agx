//! Manifest parsing, serialization, and validation for `agent-environment.yml`.

pub mod types;
pub mod validate;

use anyhow::{Context, Result};
use std::path::Path;
use types::AgentEnvironment;

/// Read and parse a manifest from a file path.
pub fn load(path: &Path) -> Result<AgentEnvironment> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: AgentEnvironment =
        serde_yaml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    Ok(manifest)
}

/// Serialize a manifest and write it to a file path.
pub fn save(manifest: &AgentEnvironment, path: &Path) -> Result<()> {
    let content = serde_yaml::to_string(manifest).context("serializing manifest")?;
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
