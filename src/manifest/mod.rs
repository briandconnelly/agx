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

    #[test]
    fn load_invalid_yaml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-environment.yml");
        std::fs::write(&path, "{{{{not: valid: yaml: [[[").unwrap();

        let result = load(&path);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("parsing"),
            "expected parsing error, got: {msg}"
        );
    }

    #[test]
    fn load_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.yml");

        let result = load(&path);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("reading"),
            "expected reading error, got: {msg}"
        );
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-environment.yml");

        let manifest = AgentEnvironment {
            name: "roundtrip-test".to_string(),
            version: Some("2.0.0".to_string()),
            description: Some("A roundtrip test".to_string()),
            backends: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "claude-code".to_string(),
                    types::BackendConfig {
                        default: Some(true),
                        isolation: Some(types::IsolationMode::Strict),
                        model: None,
                    },
                );
                m
            },
            registries: std::collections::BTreeMap::new(),
            plugins: std::collections::BTreeMap::new(),
            skills: std::collections::BTreeMap::new(),
            agents: std::collections::BTreeMap::new(),
            mcp_servers: std::collections::BTreeMap::new(),
            settings: std::collections::BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };

        save(&manifest, &path).unwrap();

        // Verify file exists and has content
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("roundtrip-test"));
        assert!(content.contains("2.0.0"));

        // Reload and verify
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.name, "roundtrip-test");
        assert_eq!(loaded.version.as_deref(), Some("2.0.0"));
        assert_eq!(loaded.description.as_deref(), Some("A roundtrip test"));
        assert!(loaded.backends.contains_key("claude-code"));
    }

    #[test]
    fn save_creates_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-environment.yml");

        let manifest = AgentEnvironment {
            name: "yaml-check".to_string(),
            version: None,
            description: None,
            backends: std::collections::BTreeMap::new(),
            registries: std::collections::BTreeMap::new(),
            plugins: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "my-plugin".to_string(),
                    types::PluginDependency {
                        registry: Some("official".to_string()),
                        version: Some("^1.0".to_string()),
                        git: None,
                        tag: None,
                        branch: None,
                        commit: None,
                        include: None,
                        exclude: None,
                    },
                );
                m
            },
            skills: std::collections::BTreeMap::new(),
            agents: std::collections::BTreeMap::new(),
            mcp_servers: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "fs".to_string(),
                    types::McpServerConfig {
                        url: None,
                        command: Some("npx".to_string()),
                        args: vec!["@server/fs".to_string()],
                        env: std::collections::BTreeMap::new(),
                    },
                );
                m
            },
            settings: std::collections::BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        };

        save(&manifest, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.plugins.len(), 1);
        assert_eq!(loaded.mcp_servers.len(), 1);
        assert_eq!(
            loaded.plugins["my-plugin"].version.as_deref(),
            Some("^1.0")
        );
    }

    #[test]
    fn load_valid_minimal_manifest_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-environment.yml");
        std::fs::write(&path, "name: minimal\n").unwrap();

        let env = load(&path).unwrap();
        assert_eq!(env.name, "minimal");
        assert!(env.plugins.is_empty());
        assert!(env.backends.is_empty());
    }
}

/// Serialize a manifest and write it to a file path.
pub fn save(manifest: &AgentEnvironment, path: &Path) -> Result<()> {
    let content = serde_yaml::to_string(manifest).context("serializing manifest")?;
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
