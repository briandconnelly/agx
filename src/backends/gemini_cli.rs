//! Gemini CLI backend — generates `.gemini/`, `GEMINI.md`.

use super::{
    copy_agents, copy_skills, yaml_to_json, Backend, ResolvedEnvironment, GENERATED_MARKER,
};
use crate::manifest::types::{AgentDependency, BackendConfig, ModelConfig, ModelSpec};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Gemini CLI backend implementation.
///
/// Generates the full Gemini CLI environment from a resolved manifest:
/// `.gemini/settings.json`, `.gemini/agents/*.md`, `.gemini/skills/*/`.
/// MCP servers are not supported — a warning is emitted if configured.
pub struct GeminiCliBackend;

impl Backend for GeminiCliBackend {
    fn name(&self) -> &str {
        "gemini-cli"
    }

    fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()> {
        let manifest = resolved.manifest;
        let backend_config = manifest.backends.get("gemini-cli");
        let settings = manifest.settings.get("gemini-cli");

        // Generate .gemini/settings.json
        generate_settings(backend_config, settings, &manifest.agents, output_dir)
            .context("generating .gemini/settings.json")?;

        // Generate .gemini/agents/*.md
        let agents_dir = output_dir.join(".gemini").join("agents");
        copy_agents(&manifest.agents, resolved.cache_dir, output_dir, &agents_dir)
            .context("generating .gemini/agents/")?;

        // Generate .gemini/skills/*/
        let skills_dir = output_dir.join(".gemini").join("skills");
        copy_skills(&manifest.skills, resolved.cache_dir, output_dir, &skills_dir)
            .context("generating .gemini/skills/")?;

        // Warn about MCP servers — Gemini CLI doesn't use MCP.
        if !manifest.mcp_servers.is_empty() {
            eprintln!(
                "warning: gemini-cli backend does not support MCP servers; \
                 {} server(s) configured in manifest will be skipped for this backend",
                manifest.mcp_servers.len()
            );
        }

        Ok(())
    }

    fn context_filename(&self) -> &str {
        "GEMINI.md"
    }

    fn run_command(&self, config: &BackendConfig) -> Result<Command> {
        let mut cmd = Command::new("gemini");

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

/// Map a snake_case parameter name to Gemini CLI's camelCase equivalent.
fn map_param_name(name: &str) -> &str {
    match name {
        "top_p" => "topP",
        "max_output_tokens" => "maxOutputTokens",
        "thinking_budget" => "thinkingBudget",
        // These are already correct or pass through as-is.
        _ => name,
    }
}

/// Convert a `serde_yaml::Value` to a `serde_json::Value` suitable for
/// Gemini's `modelConfigs` numeric fields.
fn yaml_val_to_json(v: &serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                json!(i)
            } else if let Some(f) = n.as_f64() {
                json!(f)
            } else {
                Value::Null
            }
        }
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::String(s) => json!(s),
        other => yaml_to_json(other),
    }
}

/// Build the `modelConfigs` object from the backend-level model config and
/// per-agent parameter overrides.
fn build_model_configs(
    model_config: Option<&ModelConfig>,
    agents: &BTreeMap<String, AgentDependency>,
) -> Option<Value> {
    let has_backend_params = model_config.map_or(false, |c| !c.parameters.is_empty());
    let has_agent_overrides = agents
        .values()
        .any(|a| !a.parameters.is_empty() || a.model.is_some());

    if !has_backend_params && !has_agent_overrides {
        return None;
    }

    let mut configs = Map::new();

    // Backend-level model parameter overrides, keyed by model ID.
    if let Some(cfg) = model_config {
        if !cfg.parameters.is_empty() {
            let mut params_obj = Map::new();
            for (k, v) in &cfg.parameters {
                params_obj.insert(map_param_name(k).to_string(), yaml_val_to_json(v));
            }
            configs.insert(cfg.id.clone(), Value::Object(params_obj));
        }
    }

    // Per-agent overrides go under `overrides.<agent_name>`.
    let mut overrides = Map::new();
    for (name, agent) in agents {
        if agent.parameters.is_empty() && agent.model.is_none() {
            continue;
        }
        let mut agent_override = Map::new();
        if let Some(ref model) = agent.model {
            agent_override.insert("model".to_string(), json!(model));
        }
        for (k, v) in &agent.parameters {
            agent_override.insert(map_param_name(k).to_string(), yaml_val_to_json(v));
        }
        overrides.insert(name.clone(), Value::Object(agent_override));
    }

    if !overrides.is_empty() {
        configs.insert("overrides".to_string(), Value::Object(overrides));
    }

    Some(Value::Object(configs))
}

/// Generate `.gemini/settings.json` with model preferences, `modelConfigs`,
/// and any user-supplied settings from the manifest's `settings.gemini-cli` section.
fn generate_settings(
    backend_config: Option<&BackendConfig>,
    settings: Option<&serde_yaml::Value>,
    agents: &BTreeMap<String, AgentDependency>,
    output_dir: &Path,
) -> Result<()> {
    let mut obj = Map::new();
    obj.insert(
        "_generated_by".to_string(),
        Value::String(GENERATED_MARKER.to_string()),
    );

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

    // Add model preferences if specified.
    let mut model_config_ref: Option<&ModelConfig> = None;
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
                    model_config_ref = Some(cfg);
                }
            }
        }
    }

    // Build modelConfigs from backend parameters + per-agent overrides.
    if let Some(model_configs) = build_model_configs(model_config_ref, agents) {
        obj.insert("modelConfigs".to_string(), model_configs);
    }

    let settings_dir = output_dir.join(".gemini");
    fs::create_dir_all(&settings_dir).context("creating .gemini directory")?;

    let json_output = serde_json::to_string_pretty(&Value::Object(obj))?;
    fs::write(settings_dir.join("settings.json"), json_output + "\n")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ResolvedEnvironment;
    use crate::lockfile::types::Lockfile;
    use crate::manifest::types::{
        AgentEnvironment, IsolationMode, McpServerConfig, ModelConfig, SkillDependency,
    };
    use tempfile::tempdir;

    // -----------------------------------------------------------------------
    // Settings generation
    // -----------------------------------------------------------------------

    #[test]
    fn test_settings_basic() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let agents = BTreeMap::new();
        generate_settings(Some(&config), None, &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["_generated_by"], GENERATED_MARKER);
    }

    #[test]
    fn test_settings_with_simple_model() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Simple("gemini-2.5-pro".to_string())),
        };
        let agents = BTreeMap::new();
        generate_settings(Some(&config), None, &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["model"], "gemini-2.5-pro");
    }

    #[test]
    fn test_settings_with_full_model_and_params() {
        let dir = tempdir().unwrap();
        let mut params = BTreeMap::new();
        params.insert(
            "temperature".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0.7_f64)),
        );
        params.insert(
            "top_p".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0.95_f64)),
        );
        params.insert(
            "max_output_tokens".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(8192)),
        );
        params.insert(
            "thinking_budget".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(4096)),
        );

        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Full(ModelConfig {
                id: "gemini-2.5-pro".to_string(),
                extended_context: None,
                effort: None,
                plan_model: None,
                available: vec![],
                parameters: params,
            })),
        };
        let agents = BTreeMap::new();
        generate_settings(Some(&config), None, &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["model"], "gemini-2.5-pro");

        let model_configs = &parsed["modelConfigs"]["gemini-2.5-pro"];
        assert_eq!(model_configs["temperature"], 0.7);
        assert_eq!(model_configs["topP"], 0.95);
        assert_eq!(model_configs["maxOutputTokens"], 8192);
        assert_eq!(model_configs["thinkingBudget"], 4096);
    }

    #[test]
    fn test_parameter_name_mapping() {
        assert_eq!(map_param_name("temperature"), "temperature");
        assert_eq!(map_param_name("top_p"), "topP");
        assert_eq!(map_param_name("max_output_tokens"), "maxOutputTokens");
        assert_eq!(map_param_name("thinking_budget"), "thinkingBudget");
        assert_eq!(map_param_name("include_thoughts"), "include_thoughts");
    }

    #[test]
    fn test_settings_with_user_settings() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let yaml_settings: serde_yaml::Value = serde_yaml::from_str(
            r#"
sandbox: true
codeExecution:
  enabled: true
"#,
        )
        .unwrap();

        let agents = BTreeMap::new();
        generate_settings(Some(&config), Some(&yaml_settings), &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["sandbox"], true);
        assert_eq!(parsed["codeExecution"]["enabled"], true);
    }

    #[test]
    fn test_settings_with_per_agent_overrides() {
        let dir = tempdir().unwrap();

        let mut backend_params = BTreeMap::new();
        backend_params.insert(
            "temperature".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0.7_f64)),
        );

        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Full(ModelConfig {
                id: "gemini-2.5-pro".to_string(),
                extended_context: None,
                effort: None,
                plan_model: None,
                available: vec![],
                parameters: backend_params,
            })),
        };

        let mut agent_params = BTreeMap::new();
        agent_params.insert(
            "temperature".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0.1_f64)),
        );
        agent_params.insert(
            "thinking_budget".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(8192)),
        );

        let mut agents = BTreeMap::new();
        agents.insert(
            "security-scanner".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/scanner.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("gemini-2.5-pro".to_string()),
                parameters: agent_params,
            },
        );

        generate_settings(Some(&config), None, &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        // Backend-level config.
        assert_eq!(parsed["modelConfigs"]["gemini-2.5-pro"]["temperature"], 0.7);

        // Per-agent override.
        let agent_override = &parsed["modelConfigs"]["overrides"]["security-scanner"];
        assert_eq!(agent_override["model"], "gemini-2.5-pro");
        assert_eq!(agent_override["temperature"], 0.1);
        assert_eq!(agent_override["thinkingBudget"], 8192);
    }

    // -----------------------------------------------------------------------
    // Agent frontmatter injection
    // -----------------------------------------------------------------------

    #[test]
    fn test_agent_frontmatter_injection() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create local agent file.
        let agents_src = dir.path().join("agents");
        fs::create_dir_all(&agents_src).unwrap();
        fs::write(
            agents_src.join("reviewer.md"),
            "---\ndescription: Code reviewer\n---\nReview the code.\n",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "reviewer".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/reviewer.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("gemini-2.5-pro".to_string()),
                parameters: BTreeMap::new(),
            },
        );

        let agents_dir = dir.path().join(".gemini").join("agents");
        copy_agents(&agents, cache.path(), dir.path(), &agents_dir).unwrap();

        let output = fs::read_to_string(dir.path().join(".gemini/agents/reviewer.md")).unwrap();
        assert!(output.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(output.contains("model: gemini-2.5-pro"));
        assert!(output.contains("description: Code reviewer"));
        assert!(output.contains("Review the code."));
    }

    #[test]
    fn test_agent_with_parameters_in_frontmatter() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        let agents_src = dir.path().join("agents");
        fs::create_dir_all(&agents_src).unwrap();
        fs::write(agents_src.join("scanner.md"), "Scan for issues.\n").unwrap();

        let mut params = BTreeMap::new();
        params.insert(
            "temperature".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0.1_f64)),
        );

        let mut agents = BTreeMap::new();
        agents.insert(
            "scanner".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/scanner.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("gemini-2.5-pro".to_string()),
                parameters: params,
            },
        );

        let agents_dir = dir.path().join(".gemini").join("agents");
        copy_agents(&agents, cache.path(), dir.path(), &agents_dir).unwrap();

        let output = fs::read_to_string(dir.path().join(".gemini/agents/scanner.md")).unwrap();
        assert!(output.contains("model: gemini-2.5-pro"));
        assert!(output.contains("temperature: 0.1"));
        assert!(output.contains("Scan for issues."));
    }

    // -----------------------------------------------------------------------
    // Skill directory copying
    // -----------------------------------------------------------------------

    #[test]
    fn test_skill_copy_local() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

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

        let skills_dir = dir.path().join(".gemini").join("skills");
        copy_skills(&skills, cache.path(), dir.path(), &skills_dir).unwrap();

        let skill_md =
            fs::read_to_string(dir.path().join(".gemini/skills/deploy-checklist/SKILL.md"))
                .unwrap();
        assert!(skill_md.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(skill_md.contains("# Deploy Checklist"));

        let extra =
            fs::read_to_string(dir.path().join(".gemini/skills/deploy-checklist/extra.txt"))
                .unwrap();
        assert_eq!(extra, "extra content");
    }

    #[test]
    fn test_skill_copy_cached() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        // Create a cached skill.
        let cache_skill = cache.path().join("skills").join("remote-skill");
        fs::create_dir_all(&cache_skill).unwrap();
        fs::write(cache_skill.join("SKILL.md"), "# Remote Skill\n").unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "remote-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/example/skills.git".to_string()),
                path: "skills/remote-skill".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
            },
        );

        let skills_dir = dir.path().join(".gemini").join("skills");
        copy_skills(&skills, cache.path(), dir.path(), &skills_dir).unwrap();

        let skill_md =
            fs::read_to_string(dir.path().join(".gemini/skills/remote-skill/SKILL.md")).unwrap();
        assert!(skill_md.starts_with("<!-- AUTO-GENERATED BY agx. Source: cached remote-skill"));
        assert!(skill_md.contains("# Remote Skill"));
    }

    // -----------------------------------------------------------------------
    // MCP warning
    // -----------------------------------------------------------------------

    #[test]
    fn test_mcp_servers_emit_warning() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        let mut mcp_servers = BTreeMap::new();
        mcp_servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  gemini-cli:
    model: gemini-2.5-pro
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
        let backend = GeminiCliBackend;

        // Should succeed without error — MCP servers are skipped with a warning.
        backend.generate(&resolved, dir.path()).unwrap();

        // No .mcp.json should be generated.
        assert!(!dir.path().join(".mcp.json").exists());

        // Settings should still be generated.
        assert!(dir.path().join(".gemini/settings.json").exists());
    }

    // -----------------------------------------------------------------------
    // Idempotency
    // -----------------------------------------------------------------------

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
  gemini-cli:
    model: gemini-2.5-pro
agents:
  test:
    path: ./agents/test.md
    model: gemini-2.5-flash
skills:
  my-skill:
    path: ./skills/my-skill
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: cache.path(),
        };
        let backend = GeminiCliBackend;

        // Generate once.
        backend.generate(&resolved, dir.path()).unwrap();
        let settings1 = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let agent1 = fs::read_to_string(dir.path().join(".gemini/agents/test.md")).unwrap();
        let skill1 =
            fs::read_to_string(dir.path().join(".gemini/skills/my-skill/SKILL.md")).unwrap();

        // Generate again.
        backend.generate(&resolved, dir.path()).unwrap();
        let settings2 = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let agent2 = fs::read_to_string(dir.path().join(".gemini/agents/test.md")).unwrap();
        let skill2 =
            fs::read_to_string(dir.path().join(".gemini/skills/my-skill/SKILL.md")).unwrap();

        assert_eq!(settings1, settings2);
        assert_eq!(agent1, agent2);
        assert_eq!(skill1, skill2);
    }

    // -----------------------------------------------------------------------
    // Full integration
    // -----------------------------------------------------------------------

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

        // Create local skill.
        let skill_src = dir.path().join("skills").join("deploy");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "# Deploy\n").unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  gemini-cli:
    model:
      id: gemini-2.5-pro
      parameters:
        temperature: 0.7
        top_p: 0.95
agents:
  reviewer:
    path: ./agents/reviewer.md
    model: gemini-2.5-flash
skills:
  deploy:
    path: ./skills/deploy
settings:
  gemini-cli:
    sandbox: true
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: cache.path(),
        };
        let backend = GeminiCliBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        // Verify settings.json
        let settings_content =
            fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let settings: Value = serde_json::from_str(&settings_content).unwrap();
        assert_eq!(settings["_generated_by"], GENERATED_MARKER);
        assert_eq!(settings["model"], "gemini-2.5-pro");
        assert_eq!(settings["sandbox"], true);
        assert_eq!(settings["modelConfigs"]["gemini-2.5-pro"]["temperature"], 0.7);
        assert_eq!(settings["modelConfigs"]["gemini-2.5-pro"]["topP"], 0.95);

        // Per-agent override in modelConfigs.
        let reviewer_override = &settings["modelConfigs"]["overrides"]["reviewer"];
        assert_eq!(reviewer_override["model"], "gemini-2.5-flash");

        // Verify agent
        let agent = fs::read_to_string(dir.path().join(".gemini/agents/reviewer.md")).unwrap();
        assert!(agent.contains("AUTO-GENERATED BY agx"));
        assert!(agent.contains("model: gemini-2.5-flash"));
        assert!(agent.contains("description: Code reviewer"));

        // Verify skill
        let skill_md =
            fs::read_to_string(dir.path().join(".gemini/skills/deploy/SKILL.md")).unwrap();
        assert!(skill_md.contains("AUTO-GENERATED BY agx"));
        assert!(skill_md.contains("# Deploy"));
    }

    // -----------------------------------------------------------------------
    // run_command
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_command_with_model() {
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Simple("gemini-2.5-pro".to_string())),
        };
        let backend = GeminiCliBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--model")));
        assert!(args.contains(&&std::ffi::OsStr::new("gemini-2.5-pro")));
    }

    #[test]
    fn test_run_command_no_model() {
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let backend = GeminiCliBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.is_empty());
    }

    #[test]
    fn test_run_command_full_model() {
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Full(ModelConfig {
                id: "gemini-2.5-flash".to_string(),
                extended_context: None,
                effort: None,
                plan_model: None,
                available: vec![],
                parameters: BTreeMap::new(),
            })),
        };
        let backend = GeminiCliBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--model")));
        assert!(args.contains(&&std::ffi::OsStr::new("gemini-2.5-flash")));
    }

    // -----------------------------------------------------------------------
    // No modelConfigs when no params
    // -----------------------------------------------------------------------

    #[test]
    fn test_settings_no_model_configs_when_no_params() {
        let dir = tempdir().unwrap();
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: Some(ModelSpec::Simple("gemini-2.5-pro".to_string())),
        };
        let agents = BTreeMap::new();
        generate_settings(Some(&config), None, &agents, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["model"], "gemini-2.5-pro");
        // No modelConfigs when there are no parameters.
        assert!(parsed.get("modelConfigs").is_none());
    }

    // -----------------------------------------------------------------------
    // Cached agent
    // -----------------------------------------------------------------------

    #[test]
    fn test_cached_agent() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

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

        let agents_dir = dir.path().join(".gemini").join("agents");
        copy_agents(&agents, cache.path(), dir.path(), &agents_dir).unwrap();

        let output =
            fs::read_to_string(dir.path().join(".gemini/agents/remote-agent.md")).unwrap();
        assert!(output.starts_with("<!-- AUTO-GENERATED BY agx. Source: cached remote-agent -->"));
        assert!(output.contains("description: Remote agent"));
    }
}
