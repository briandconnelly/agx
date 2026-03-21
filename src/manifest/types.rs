//! Data types for the `agent-environment.yml` manifest.
//!
//! These types are the shared contract between all modules in agx.
//! They map directly to the manifest format defined in the spec.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level manifest: `agent-environment.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvironment {
    /// Project name (required).
    pub name: String,

    /// Project version (semver).
    #[serde(default)]
    pub version: Option<String>,

    /// Project description.
    #[serde(default)]
    pub description: Option<String>,

    /// Backend configurations. At least one is required.
    #[serde(default)]
    pub backends: BTreeMap<String, BackendConfig>,

    /// Named marketplace registries.
    #[serde(default)]
    pub registries: BTreeMap<String, RegistryConfig>,

    /// Plugin dependencies.
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginDependency>,

    /// Skill dependencies.
    #[serde(default)]
    pub skills: BTreeMap<String, SkillDependency>,

    /// Agent definitions.
    #[serde(default)]
    pub agents: BTreeMap<String, AgentDependency>,

    /// MCP server configurations.
    #[serde(default, rename = "mcp-servers")]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,

    /// Per-backend settings (escape hatch).
    #[serde(default)]
    pub settings: BTreeMap<String, serde_yaml::Value>,

    /// Override default local asset paths.
    #[serde(default)]
    pub paths: Option<PathsConfig>,

    /// Context file management (CLAUDE.md, GEMINI.md, etc.).
    #[serde(default)]
    pub context: Option<ContextConfig>,

    /// Lifecycle hooks (pre/post install/generate).
    #[serde(default)]
    pub hooks: Option<HooksConfig>,

    /// Path to a `.env` file to load when running backends.
    #[serde(default, rename = "env-file")]
    pub env_file: Option<String>,
}

/// Configuration for a single backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Whether this is the default backend for `agx run`.
    #[serde(default)]
    pub default: Option<bool>,

    /// Isolation mode: "strict" or "permissive".
    #[serde(default)]
    pub isolation: Option<IsolationMode>,

    /// Model configuration — either a simple string or an object.
    #[serde(default)]
    pub model: Option<ModelSpec>,
}

/// Isolation mode for a backend.
///
/// - `permissive`: project assets layer on top of user config.
/// - `strict` (default): only project-scoped assets; user/global config suppressed.
/// - `bare`: maximum isolation — skips hooks, plugins, auto-memory, etc. Ideal for CI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IsolationMode {
    Permissive,
    Strict,
    Bare,
}

impl Default for IsolationMode {
    fn default() -> Self {
        Self::Strict
    }
}

impl std::fmt::Display for IsolationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Permissive => write!(f, "permissive"),
            Self::Strict => write!(f, "strict"),
            Self::Bare => write!(f, "bare"),
        }
    }
}

/// Model specification — either a simple alias string or a full config object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelSpec {
    /// Simple model alias or ID, e.g. "sonnet", "claude-opus-4-6".
    Simple(String),
    /// Extended model configuration.
    Full(ModelConfig),
}

/// Extended model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model alias or full ID.
    pub id: String,

    /// Extended context mode (e.g. the [1m] suffix).
    #[serde(default)]
    pub extended_context: Option<bool>,

    /// Effort level: low, medium, high.
    #[serde(default)]
    pub effort: Option<String>,

    /// Model to use in plan mode (Claude Code specific).
    #[serde(default)]
    pub plan_model: Option<String>,

    /// Models available for mid-session switching.
    #[serde(default)]
    pub available: Vec<String>,

    /// Backend-specific hyperparameters (passthrough).
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_yaml::Value>,
}

/// A named marketplace registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Git repository URL for the registry.
    pub url: String,

    /// Authentication method (e.g. "token").
    #[serde(default)]
    pub auth: Option<String>,

    /// Pin the registry index to a specific commit SHA for reproducibility.
    #[serde(default)]
    pub pin: Option<String>,
}

/// A plugin dependency — from a registry or from git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDependency {
    // --- Registry source ---
    /// Name of a registry defined in `registries`.
    #[serde(default)]
    pub registry: Option<String>,

    /// Semver version constraint (for registry sources).
    #[serde(default)]
    pub version: Option<String>,

    // --- Git source ---
    /// Git repository URL.
    #[serde(default)]
    pub git: Option<String>,

    /// Git tag.
    #[serde(default)]
    pub tag: Option<String>,

    /// Git branch.
    #[serde(default)]
    pub branch: Option<String>,

    /// Full commit SHA.
    #[serde(default)]
    pub commit: Option<String>,

    // --- Asset filtering ---
    /// Include only specific assets from this plugin.
    #[serde(default)]
    pub include: Option<AssetFilter>,

    /// Exclude specific assets from this plugin.
    #[serde(default)]
    pub exclude: Option<AssetFilter>,
}

/// Filter for selecting specific assets from a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetFilter {
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

/// A skill dependency — from git or local path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDependency {
    /// Git repository URL.
    #[serde(default)]
    pub git: Option<String>,

    /// Path within the repo (for git) or local path (starts with `./`).
    pub path: String,

    /// Git tag.
    #[serde(default)]
    pub tag: Option<String>,

    /// Git branch.
    #[serde(default)]
    pub branch: Option<String>,

    /// Full commit SHA.
    #[serde(default)]
    pub commit: Option<String>,
}

/// An agent dependency — from git or local path, with optional model override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDependency {
    /// Git repository URL.
    #[serde(default)]
    pub git: Option<String>,

    /// Path within the repo (for git) or local path (starts with `./`).
    pub path: String,

    /// Git tag.
    #[serde(default)]
    pub tag: Option<String>,

    /// Git branch.
    #[serde(default)]
    pub branch: Option<String>,

    /// Full commit SHA.
    #[serde(default)]
    pub commit: Option<String>,

    /// Model override for this agent.
    #[serde(default)]
    pub model: Option<String>,

    /// Backend-specific hyperparameters for this agent (passthrough).
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_yaml::Value>,
}

/// MCP server configuration — either URL-based or command-based.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// SSE endpoint URL (for remote servers).
    #[serde(default)]
    pub url: Option<String>,

    /// Executable command (for local servers).
    #[serde(default)]
    pub command: Option<String>,

    /// Command arguments.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the server process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Override default local asset directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Directory for local agent files (default: `./agents`).
    #[serde(default)]
    pub agents: Option<String>,

    /// Directory for local skill files (default: `./skills`).
    #[serde(default)]
    pub skills: Option<String>,
}

/// Context file management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Path to the shared base context file.
    pub base: String,

    /// Per-backend overlay file paths.
    #[serde(default)]
    pub overlays: BTreeMap<String, String>,

    /// Subdirectory context files (monorepo support).
    #[serde(default)]
    pub children: Vec<ChildContext>,
}

/// A subdirectory context entry for monorepo support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildContext {
    /// Subdirectory path.
    pub path: String,

    /// Path to the base context file for this subdirectory.
    pub base: String,

    /// Per-backend overlay files for this subdirectory.
    #[serde(default)]
    pub overlays: BTreeMap<String, String>,
}

/// Lifecycle hooks configuration.
///
/// Allows running shell commands before/after install and generate operations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    /// Commands to run before the install (lock/fetch) step.
    #[serde(default, rename = "pre-install")]
    pub pre_install: Vec<String>,

    /// Commands to run after the install (generate) step.
    #[serde(default, rename = "post-install")]
    pub post_install: Vec<String>,

    /// Commands to run before backend generation.
    #[serde(default, rename = "pre-generate")]
    pub pre_generate: Vec<String>,

    /// Commands to run after backend generation.
    #[serde(default, rename = "post-generate")]
    pub post_generate: Vec<String>,
}

impl SkillDependency {
    /// Returns true if this is a local (path-based) dependency.
    pub fn is_local(&self) -> bool {
        self.path.starts_with("./") || self.path.starts_with("../")
    }
}

impl AgentDependency {
    /// Returns true if this is a local (path-based) dependency.
    pub fn is_local(&self) -> bool {
        self.path.starts_with("./") || self.path.starts_with("../")
    }
}

impl PluginDependency {
    /// Returns true if this is a registry-based dependency.
    pub fn is_registry(&self) -> bool {
        self.registry.is_some()
    }

    /// Returns true if this is a git-based dependency.
    pub fn is_git(&self) -> bool {
        self.git.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let yaml = r#"
name: test-project
backends:
  claude-code:
    isolation: strict
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(env.name, "test-project");
        assert!(env.backends.contains_key("claude-code"));
    }

    #[test]
    fn parse_simple_model() {
        let yaml = r#"
name: test
backends:
  claude-code:
    model: sonnet
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        let backend = &env.backends["claude-code"];
        match &backend.model {
            Some(ModelSpec::Simple(s)) => assert_eq!(s, "sonnet"),
            other => panic!("expected Simple model, got {:?}", other),
        }
    }

    #[test]
    fn parse_full_model() {
        let yaml = r#"
name: test
backends:
  claude-code:
    model:
      id: claude-opus-4-6
      effort: high
      plan_model: opus
      available:
        - sonnet
        - opus
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        let backend = &env.backends["claude-code"];
        match &backend.model {
            Some(ModelSpec::Full(cfg)) => {
                assert_eq!(cfg.id, "claude-opus-4-6");
                assert_eq!(cfg.effort.as_deref(), Some("high"));
                assert_eq!(cfg.plan_model.as_deref(), Some("opus"));
                assert_eq!(cfg.available.len(), 2);
            }
            other => panic!("expected Full model, got {:?}", other),
        }
    }

    #[test]
    fn parse_full_manifest() {
        let yaml = r#"
name: my-agent-project
version: 0.1.0
description: Test project

backends:
  claude-code:
    default: true
    isolation: strict
    model: sonnet
  gemini-cli:
    isolation: permissive

registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official

plugins:
  quality-review:
    registry: official
    version: ">=1.0.0, <2.0.0"
  custom-plugin:
    git: https://github.com/someone/their-plugin.git
    tag: v3.0.0

skills:
  deploy-checklist:
    path: ./skills/deploy-checklist

agents:
  my-reviewer:
    path: ./agents/my-reviewer.md
    model: opus

mcp-servers:
  playwright:
    url: https://mcp.playwright.dev/sse
  filesystem:
    command: npx
    args:
      - "@anthropic-ai/mcp-server-filesystem"
      - "./src"
    env:
      ALLOW_WRITE: "true"

context:
  base: ./CONTEXT.md
  overlays:
    claude-code: ./CONTEXT.claude-code.md
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(env.name, "my-agent-project");
        assert_eq!(env.backends.len(), 2);
        assert_eq!(env.plugins.len(), 2);
        assert_eq!(env.skills.len(), 1);
        assert_eq!(env.agents.len(), 1);
        assert_eq!(env.mcp_servers.len(), 2);
        assert!(env.agents["my-reviewer"].model.as_deref() == Some("opus"));
        assert!(env.skills["deploy-checklist"].is_local());
    }

    #[test]
    fn parse_registry_with_pin() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official
    pin: abcdef1234567890abcdef1234567890abcdef12
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            env.registries["official"].pin.as_deref(),
            Some("abcdef1234567890abcdef1234567890abcdef12")
        );
    }

    #[test]
    fn parse_registry_pin_defaults_to_none() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert!(env.registries["official"].pin.is_none());
    }

    #[test]
    fn parse_manifest_with_hooks() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
hooks:
  pre-install:
    - echo "Installing..."
  post-install:
    - ./scripts/verify.sh
  pre-generate:
    - echo "Generating..."
  post-generate:
    - ./scripts/post.sh
    - echo "Done!"
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        let hooks = env.hooks.unwrap();
        assert_eq!(hooks.pre_install, vec!["echo \"Installing...\""]);
        assert_eq!(hooks.post_install, vec!["./scripts/verify.sh"]);
        assert_eq!(hooks.pre_generate, vec!["echo \"Generating...\""]);
        assert_eq!(hooks.post_generate.len(), 2);
    }

    #[test]
    fn hooks_default_to_none_when_omitted() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert!(env.hooks.is_none());
    }

    #[test]
    fn hooks_fields_default_to_empty() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
hooks:
  pre-install:
    - echo "hi"
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        let hooks = env.hooks.unwrap();
        assert_eq!(hooks.pre_install.len(), 1);
        assert!(hooks.post_install.is_empty());
        assert!(hooks.pre_generate.is_empty());
        assert!(hooks.post_generate.is_empty());
    }

    #[test]
    fn parse_env_file_field() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
env-file: .env.production
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(env.env_file.as_deref(), Some(".env.production"));
    }

    #[test]
    fn env_file_defaults_to_none() {
        let yaml = r#"
name: test
backends:
  claude-code: {}
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert!(env.env_file.is_none());
    }
}
