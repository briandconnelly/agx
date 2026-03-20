//! Manifest parsing, serialization, and validation for `agent-environment.yml`.

pub mod types;
pub mod validate;

use anyhow::{Context, Result};
use std::path::Path;
use types::AgentEnvironment;

/// Read and parse a manifest from a file path.
///
/// Returns a clear error if the file is not valid UTF-8 or cannot be parsed.
pub fn load(path: &Path) -> Result<AgentEnvironment> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let content = String::from_utf8(bytes).map_err(|_| {
        anyhow::anyhow!(
            "manifest file is not valid UTF-8: {}",
            path.display()
        )
    })?;
    let manifest: AgentEnvironment =
        serde_yaml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_non_utf8_manifest_returns_error() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Write invalid UTF-8 bytes
        tmp.as_file()
            .write_all(&[0xFF, 0xFE, 0x00, 0x01])
            .unwrap();

        let result = load(tmp.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not valid UTF-8"),
            "expected UTF-8 error, got: {msg}"
        );
    }
}

/// Serialize a manifest and write it to a file path.
pub fn save(manifest: &AgentEnvironment, path: &Path) -> Result<()> {
    let content = serde_yaml::to_string(manifest).context("serializing manifest")?;
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
