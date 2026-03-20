//! Claude Code backend — generates `.claude/`, `.mcp.json`, `CLAUDE.md`.

use super::{
    copy_dir_recursive, inject_agent_frontmatter, resolve_env_vars, yaml_to_json, Backend,
    ResolvedEnvironment, GENERATED_MARKER,
};
use crate::manifest::types::{
    AgentDependency, BackendConfig, IsolationMode, McpServerConfig, ModelSpec, SkillDependency,
};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Claude Code backend implementation.
///
/// Generates the full Claude Code environment from a resolved manifest:
/// `.claude/settings.json`, `.claude/agents/*.md`, `.claude/skills/*/`,
/// and `.mcp.json`.
pub struct ClaudeCodeBackend;

impl Backend for ClaudeCodeBackend {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()> {
        let manifest = resolved.manifest;
        let backend_config = manifest.backends.get("claude-code");
        let settings = manifest.settings.get("claude-code");

        // Generate .claude/settings.json
        generate_settings(backend_config, settings, output_dir)
            .context("generating .claude/settings.json")?;

        // Generate .claude/agents/*.md
        generate_agents(&manifest.agents, resolved.cache_dir, output_dir)
            .context("generating .claude/agents/")?;

        // Generate .claude/skills/*/
        generate_skills(&manifest.skills, resolved.cache_dir, output_dir)
            .context("generating .claude/skills/")?;

        // Generate .mcp.json
        generate_mcp_json(&manifest.mcp_servers, output_dir).context("generating .mcp.json")?;

        Ok(())
    }

    fn context_filename(&self) -> &str {
        "CLAUDE.md"
    }

    fn run_command(&self, config: &BackendConfig) -> Result<Command> {
        let mut cmd = Command::new("claude");

        // Apply isolation flags for strict mode.
        let isolation = config.isolation.clone().unwrap_or_default();
        if isolation == IsolationMode::Strict {
            cmd.arg("--no-user-config");
        }

        // Set model if specified.
        if let Some(ref model) = config.model {
            let model_id = match model {
                ModelSpec::Simple(s) => s.clone(),
                ModelSpec::Full(cfg) => cfg.id.clone(),
            };
            cmd.args(["--model", &model_id]);
        }

        Ok(cmd)
    }
}

/// Generate `.claude/settings.json` with isolation config, permissions, and
/// any user-supplied settings from the manifest's `settings.claude-code` section.
fn generate_settings(
    backend_config: Option<&BackendConfig>,
    settings: Option<&serde_yaml::Value>,
    output_dir: &Path,
) -> Result<()> {
    let mut obj = Map::new();
    obj.insert(
        "_generated_by".to_string(),
        Value::String(GENERATED_MARKER.to_string()),
    );

    let isolation = backend_config
        .and_then(|c| c.isolation.clone())
        .unwrap_or_default();

    // Merge user-supplied settings from manifest.
    if let Some(yaml_val) = settings {
        let json_str =
            serde_json::to_string(&yaml_to_json(yaml_val)).context("converting settings YAML")?;
        let parsed: Value = serde_json::from_str(&json_str)?;
        if let Value::Object(user_obj) = parsed {
            for (k, v) in user_obj {
                obj.insert(k, v);
            }
        }
    }

    // In strict mode, add environment flags to disable user-level config.
    if isolation == IsolationMode::Strict {
        let env_obj = obj
            .entry("environment".to_string())
            .or_insert_with(|| json!({}));
        if let Value::Object(env_map) = env_obj {
            env_map
                .entry("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS")
                .or_insert_with(|| json!("1"));
        }
    }

    // Add model preferences if specified.
    if let Some(config) = backend_config {
        if let Some(ref model) = config.model {
            match model {
                ModelSpec::Simple(s) => {
                    obj.entry("model".to_string())
                        .or_insert_with(|| json!(s));
                }
                ModelSpec::Full(cfg) => {
                    obj.entry("model".to_string())
                        .or_insert_with(|| json!(cfg.id));
                    if let Some(ref effort) = cfg.effort {
                        obj.entry("effort".to_string())
                            .or_insert_with(|| json!(effort));
                    }
                    if let Some(ref plan_model) = cfg.plan_model {
                        obj.entry("planModel".to_string())
                            .or_insert_with(|| json!(plan_model));
                    }
                }
            }
        }
    }

    let settings_dir = output_dir.join(".claude");
    fs::create_dir_all(&settings_dir).context("creating .claude directory")?;

    let json_output = serde_json::to_string_pretty(&Value::Object(obj))?;
    fs::write(settings_dir.join("settings.json"), json_output + "\n")?;

    Ok(())
}

/// Generate `.claude/agents/*.md` — copy agent files from local paths or cache,
/// injecting YAML frontmatter with model/parameter overrides.
fn generate_agents(
    agents: &BTreeMap<String, AgentDependency>,
    cache_dir: &Path,
    output_dir: &Path,
) -> Result<()> {
    if agents.is_empty() {
        return Ok(());
    }

    let agents_dir = output_dir.join(".claude").join("agents");
    fs::create_dir_all(&agents_dir).context("creating .claude/agents directory")?;

    for (name, agent) in agents {
        let source_path = if agent.is_local() {
            output_dir.join(&agent.path)
        } else {
            // Cached agent: look in cache_dir/agents/<name>.md
            cache_dir.join("agents").join(format!("{name}.md"))
        };

        let content = fs::read_to_string(&source_path)
            .with_context(|| format!("reading agent source: {}", source_path.display()))?;

        let injected = inject_agent_frontmatter(&content, agent.model.as_deref(), &agent.parameters);
        let source_label = if agent.is_local() {
            format!("local {}", agent.path)
        } else {
            format!("cached {name}")
        };
        let marker = format!("<!-- AUTO-GENERATED BY agx. Source: {source_label} -->\n");

        let output_filename = if agent.path.ends_with(".md") {
            // Use the filename from the path.
            Path::new(&agent.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            format!("{name}.md")
        };

        fs::write(agents_dir.join(&output_filename), format!("{marker}{injected}"))?;
    }

    Ok(())
}

/// Generate `.claude/skills/*/` — copy skill directories from local paths or cache.
fn generate_skills(
    skills: &BTreeMap<String, SkillDependency>,
    cache_dir: &Path,
    output_dir: &Path,
) -> Result<()> {
    if skills.is_empty() {
        return Ok(());
    }

    let skills_dir = output_dir.join(".claude").join("skills");
    fs::create_dir_all(&skills_dir).context("creating .claude/skills directory")?;

    for (name, skill) in skills {
        let source_dir = if skill.is_local() {
            output_dir.join(&skill.path)
        } else {
            cache_dir.join("skills").join(name)
        };

        let target_dir = skills_dir.join(name);
        copy_dir_recursive(&source_dir, &target_dir)
            .with_context(|| format!("copying skill '{name}' from {}", source_dir.display()))?;

        // Add marker to SKILL.md if it exists.
        let skill_md = target_dir.join("SKILL.md");
        if skill_md.exists() {
            let content = fs::read_to_string(&skill_md)?;
            let source_label = if skill.is_local() {
                format!("local {}", skill.path)
            } else {
                format!("cached {name}")
            };
            let marker =
                format!("<!-- AUTO-GENERATED BY agx. Source: {source_label} -->\n");
            // Only add marker if not already present.
            if !content.starts_with("<!-- AUTO-GENERATED BY agx.") {
                fs::write(&skill_md, format!("{marker}{content}"))?;
            }
        }
    }

    Ok(())
}

/// Generate `.mcp.json` at the project root from manifest MCP server declarations.
/// Resolves `${VAR_NAME}` references in env values from the current shell environment.
fn generate_mcp_json(
    servers: &BTreeMap<String, McpServerConfig>,
    output_dir: &Path,
) -> Result<()> {
    let mut mcp_servers = Map::new();

    for (name, config) in servers {
        let mut server = Map::new();

        if let Some(ref url) = config.url {
            server.insert("url".to_string(), json!(url));
        } else if let Some(ref command) = config.command {
            server.insert("command".to_string(), json!(command));
            if !config.args.is_empty() {
                server.insert("args".to_string(), json!(config.args));
            }
        }

        if !config.env.is_empty() {
            let mut env_map = Map::new();
            for (k, v) in &config.env {
                env_map.insert(k.clone(), json!(resolve_env_vars(v)));
            }
            server.insert("env".to_string(), Value::Object(env_map));
        }

        mcp_servers.insert(name.clone(), Value::Object(server));
    }

    let mut root = Map::new();
    root.insert(
        "_generated_by".to_string(),
        Value::String(GENERATED_MARKER.to_string()),
    );
    root.insert("mcpServers".to_string(), Value::Object(mcp_servers));

    let json_output = serde_json::to_string_pretty(&Value::Object(root))?;
    fs::write(output_dir.join(".mcp.json"), json_output + "\n")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::parse_frontmatter;
    use crate::lockfile::types::Lockfile;
    use crate::manifest::types::{AgentEnvironment, IsolationMode, ModelConfig, ModelSpec};
    use tempfile::tempdir;

    /// Helper to create a minimal manifest.
    fn minimal_manifest() -> AgentEnvironment {
        serde_yaml::from_str(
            r#"
name: test-project
backends:
  claude-code:
    isolation: strict
"#,
        )
        .unwrap()
    }

    #[test]
    fn test_settings_strict_isolation() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        generate_settings(Some(&config), None, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["_generated_by"], GENERATED_MARKER);
        assert_eq!(
            parsed["environment"]["CLAUDE_CODE_DISABLE_BACKGROUND_TASKS"],
            "1"
        );
    }

    #[test]
    fn test_settings_permissive_no_env_flags() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        generate_settings(Some(&config), None, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert!(parsed.get("environment").is_none());
    }

    #[test]
    fn test_settings_with_model_simple() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Simple("sonnet".to_string())),
        };
        generate_settings(Some(&config), None, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["model"], "sonnet");
    }

    #[test]
    fn test_settings_with_model_full() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Full(ModelConfig {
                id: "claude-opus-4-6".to_string(),
                extended_context: None,
                effort: Some("high".to_string()),
                plan_model: Some("sonnet".to_string()),
                available: vec![],
                parameters: BTreeMap::new(),
            })),
        };
        generate_settings(Some(&config), None, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["model"], "claude-opus-4-6");
        assert_eq!(parsed["effort"], "high");
        assert_eq!(parsed["planModel"], "sonnet");
    }

    #[test]
    fn test_settings_with_user_settings() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        let yaml_settings: serde_yaml::Value = serde_yaml::from_str(
            r#"
permissions:
  allow:
    - "Bash(npm test)"
  deny:
    - "Bash(rm -rf)"
"#,
        )
        .unwrap();

        generate_settings(Some(&config), Some(&yaml_settings), dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["permissions"]["allow"][0], "Bash(npm test)");
        assert_eq!(parsed["permissions"]["deny"][0], "Bash(rm -rf)");
    }

    #[test]
    fn test_mcp_json_url_server() {
        let dir = tempdir().unwrap();
        let mut servers = BTreeMap::new();
        servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );

        generate_mcp_json(&servers, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            parsed["mcpServers"]["playwright"]["url"],
            "https://mcp.playwright.dev/sse"
        );
    }

    #[test]
    fn test_mcp_json_command_server() {
        let dir = tempdir().unwrap();
        let mut servers = BTreeMap::new();
        let mut env = BTreeMap::new();
        env.insert("ALLOW_WRITE".to_string(), "true".to_string());
        servers.insert(
            "filesystem".to_string(),
            McpServerConfig {
                url: None,
                command: Some("npx".to_string()),
                args: vec![
                    "@anthropic-ai/mcp-server-filesystem".to_string(),
                    "./src".to_string(),
                ],
                env,
            },
        );

        generate_mcp_json(&servers, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        let fs_server = &parsed["mcpServers"]["filesystem"];
        assert_eq!(fs_server["command"], "npx");
        assert_eq!(fs_server["args"][0], "@anthropic-ai/mcp-server-filesystem");
        assert_eq!(fs_server["args"][1], "./src");
        assert_eq!(fs_server["env"]["ALLOW_WRITE"], "true");
    }

    #[test]
    fn test_mcp_json_env_var_resolution() {
        let dir = tempdir().unwrap();
        // SAFETY: This test is single-threaded and we remove the var at the end.
        unsafe { std::env::set_var("AGX_TEST_SECRET", "my-secret-value") };

        let mut servers = BTreeMap::new();
        let mut env = BTreeMap::new();
        env.insert("API_KEY".to_string(), "${AGX_TEST_SECRET}".to_string());
        servers.insert(
            "test-server".to_string(),
            McpServerConfig {
                url: None,
                command: Some("node".to_string()),
                args: vec!["server.js".to_string()],
                env,
            },
        );

        generate_mcp_json(&servers, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            parsed["mcpServers"]["test-server"]["env"]["API_KEY"],
            "my-secret-value"
        );

        // SAFETY: Cleaning up test env var.
        unsafe { std::env::remove_var("AGX_TEST_SECRET") };
    }

    #[test]
    fn test_env_var_resolution_unset() {
        let result = resolve_env_vars("prefix-${NONEXISTENT_AGX_VAR_XYZ}-suffix");
        assert_eq!(result, "prefix--suffix");
    }

    #[test]
    fn test_env_var_resolution_no_vars() {
        let result = resolve_env_vars("plain string");
        assert_eq!(result, "plain string");
    }

    #[test]
    fn test_agent_frontmatter_injection_no_existing() {
        let content = "Agent content here\n";
        let result = inject_agent_frontmatter(content, Some("opus"), &BTreeMap::new());
        assert!(result.contains("---\nmodel: opus\n---\n"));
        assert!(result.contains("Agent content here"));
    }

    #[test]
    fn test_agent_frontmatter_injection_with_existing() {
        let content = "---\ndescription: My agent\n---\nAgent content here\n";
        let result = inject_agent_frontmatter(content, Some("opus"), &BTreeMap::new());
        assert!(result.contains("description: My agent"));
        assert!(result.contains("model: opus"));
        assert!(result.contains("Agent content here"));
    }

    #[test]
    fn test_agent_frontmatter_injection_override_model() {
        let content = "---\ndescription: My agent\nmodel: sonnet\n---\nContent\n";
        let result = inject_agent_frontmatter(content, Some("opus"), &BTreeMap::new());
        assert!(result.contains("model: opus"));
        assert!(!result.contains("model: sonnet"));
        assert!(result.contains("description: My agent"));
    }

    #[test]
    fn test_agent_frontmatter_no_injection_needed() {
        let content = "---\ndescription: My agent\n---\nContent\n";
        let result = inject_agent_frontmatter(content, None, &BTreeMap::new());
        assert_eq!(result, content);
    }

    #[test]
    fn test_agent_copy_local() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create local agent file.
        let agents_src = dir.path().join("agents");
        fs::create_dir_all(&agents_src).unwrap();
        fs::write(
            agents_src.join("my-agent.md"),
            "---\ndescription: Test\n---\nAgent body\n",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "my-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/my-agent.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: BTreeMap::new(),
            },
        );

        generate_agents(&agents, cache.path(), dir.path()).unwrap();

        let output = fs::read_to_string(dir.path().join(".claude/agents/my-agent.md")).unwrap();
        assert!(output.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(output.contains("model: opus"));
        assert!(output.contains("description: Test"));
        assert!(output.contains("Agent body"));
    }

    #[test]
    fn test_skill_copy_local() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create local skill directory.
        let skill_src = dir.path().join("skills").join("deploy-checklist");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "# Deploy Checklist\n").unwrap();
        fs::write(skill_src.join("extra.txt"), "extra content").unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "deploy-checklist".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/deploy-checklist".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        generate_skills(&skills, cache.path(), dir.path()).unwrap();

        let skill_md =
            fs::read_to_string(dir.path().join(".claude/skills/deploy-checklist/SKILL.md"))
                .unwrap();
        assert!(skill_md.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(skill_md.contains("# Deploy Checklist"));

        let extra =
            fs::read_to_string(dir.path().join(".claude/skills/deploy-checklist/extra.txt"))
                .unwrap();
        assert_eq!(extra, "extra content");
    }

    #[test]
    fn test_generate_idempotent() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create local agent.
        let agents_src = dir.path().join("agents");
        fs::create_dir_all(&agents_src).unwrap();
        fs::write(agents_src.join("test.md"), "Agent content\n").unwrap();

        // Create local skill.
        let skill_src = dir.path().join("skills").join("my-skill");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "Skill content\n").unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  claude-code:
    isolation: strict
    model: sonnet
agents:
  test:
    path: ./agents/test.md
    model: opus
skills:
  my-skill:
    path: ./skills/my-skill
mcp-servers:
  playwright:
    url: https://mcp.playwright.dev/sse
settings:
  claude-code:
    permissions:
      allow:
        - "Bash(npm test)"
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: cache.path(),
        };
        let backend = ClaudeCodeBackend;

        // Generate once.
        backend.generate(&resolved, dir.path()).unwrap();
        let settings1 = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let mcp1 = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let agent1 = fs::read_to_string(dir.path().join(".claude/agents/test.md")).unwrap();
        let skill1 =
            fs::read_to_string(dir.path().join(".claude/skills/my-skill/SKILL.md")).unwrap();

        // Generate again.
        backend.generate(&resolved, dir.path()).unwrap();
        let settings2 = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
        let mcp2 = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let agent2 = fs::read_to_string(dir.path().join(".claude/agents/test.md")).unwrap();
        let skill2 =
            fs::read_to_string(dir.path().join(".claude/skills/my-skill/SKILL.md")).unwrap();

        assert_eq!(settings1, settings2);
        assert_eq!(mcp1, mcp2);
        assert_eq!(agent1, agent2);
        assert_eq!(skill1, skill2);
    }

    #[test]
    fn test_full_generate() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create local agent.
        let agents_src = dir.path().join("agents");
        fs::create_dir_all(&agents_src).unwrap();
        fs::write(
            agents_src.join("reviewer.md"),
            "---\ndescription: Code reviewer\n---\nReview the code.\n",
        )
        .unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  claude-code:
    isolation: strict
    model: sonnet
agents:
  reviewer:
    path: ./agents/reviewer.md
    model: opus
mcp-servers:
  playwright:
    url: https://mcp.playwright.dev/sse
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: cache.path(),
        };
        let backend = ClaudeCodeBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        // Verify settings.json
        assert!(dir.path().join(".claude/settings.json").exists());

        // Verify agent
        let agent = fs::read_to_string(dir.path().join(".claude/agents/reviewer.md")).unwrap();
        assert!(agent.contains("AUTO-GENERATED BY agx"));
        assert!(agent.contains("model: opus"));
        assert!(agent.contains("description: Code reviewer"));

        // Verify .mcp.json
        let mcp = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&mcp).unwrap();
        assert_eq!(
            parsed["mcpServers"]["playwright"]["url"],
            "https://mcp.playwright.dev/sse"
        );
    }

    #[test]
    fn test_run_command_strict() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Simple("sonnet".to_string())),
        };
        let backend = ClaudeCodeBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--no-user-config")));
        assert!(args.contains(&&std::ffi::OsStr::new("--model")));
        assert!(args.contains(&&std::ffi::OsStr::new("sonnet")));
    }

    #[test]
    fn test_run_command_permissive() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let backend = ClaudeCodeBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(!args.contains(&&std::ffi::OsStr::new("--no-user-config")));
    }

    #[test]
    fn test_parse_frontmatter_present() {
        let content = "---\nfoo: bar\nbaz: 42\n---\nBody text\n";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.unwrap(), "foo: bar\nbaz: 42");
        assert_eq!(body, "Body text\n");
    }

    #[test]
    fn test_parse_frontmatter_absent() {
        let content = "Just plain text\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, "Just plain text\n");
    }

    #[test]
    fn test_mcp_json_mixed_servers() {
        let dir = tempdir().unwrap();
        let mut servers = BTreeMap::new();
        servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let mut env = BTreeMap::new();
        env.insert("ALLOW_WRITE".to_string(), "true".to_string());
        servers.insert(
            "filesystem".to_string(),
            McpServerConfig {
                url: None,
                command: Some("npx".to_string()),
                args: vec![
                    "@anthropic-ai/mcp-server-filesystem".to_string(),
                    "./src".to_string(),
                ],
                env,
            },
        );

        generate_mcp_json(&servers, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        // Both servers present.
        assert!(parsed["mcpServers"]["playwright"].is_object());
        assert!(parsed["mcpServers"]["filesystem"].is_object());
        // Generated marker present.
        assert_eq!(parsed["_generated_by"], GENERATED_MARKER);
    }

    #[test]
    fn test_empty_mcp_servers() {
        let dir = tempdir().unwrap();
        let servers = BTreeMap::new();
        generate_mcp_json(&servers, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["mcpServers"].is_object());
        assert_eq!(parsed["mcpServers"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn test_cached_agent() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create cached agent in the cache dir.
        let cache_agents = cache.path().join("agents");
        fs::create_dir_all(&cache_agents).unwrap();
        fs::write(
            cache_agents.join("remote-agent.md"),
            "---\ndescription: Remote agent\n---\nRemote agent body\n",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "remote-agent".to_string(),
            AgentDependency {
                git: Some("https://github.com/example/agents.git".to_string()),
                path: "agents/remote-agent.md".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );

        generate_agents(&agents, cache.path(), dir.path()).unwrap();

        let output =
            fs::read_to_string(dir.path().join(".claude/agents/remote-agent.md")).unwrap();
        assert!(output.starts_with("<!-- AUTO-GENERATED BY agx. Source: cached remote-agent -->"));
        assert!(output.contains("description: Remote agent"));
    }
}
