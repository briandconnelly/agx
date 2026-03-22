//! `agx import` — reverse-engineer an `agent-environment.yml` from existing config files.
//!
//! Scans a project directory for backend-specific agent configuration (`.claude/`,
//! `.gemini/`, `.mcp.json`, etc.) and generates (or merges into) a manifest.
//! This gives users instant value without writing YAML from scratch.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::Confirm;

use agx::config::MANIFEST_FILE;
use agx::manifest::types::{
    AgentDependency, AgentEnvironment, BackendConfig, ContextConfig, IsolationMode, ModelConfig,
    ModelSpec, SkillDependency,
};

use super::init::{scan_project, split_claude_md, update_gitignore, ScanResult};
use super::install;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

/// Import existing agent configuration into an `agent-environment.yml` manifest.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Non-interactive mode: accept all detected items without confirmation.
    #[arg(short = 'y', long = "yes")]
    pub non_interactive: bool,

    /// When merging into an existing manifest, overwrite conflicting entries.
    #[arg(long)]
    pub overwrite: bool,

    /// Scan and print what would be imported without writing any files.
    #[arg(long)]
    pub dry_run: bool,

    /// Only import from specific backends (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub backend: Vec<String>,

    /// Skip running `agx install` after writing the manifest.
    #[arg(long)]
    pub no_install: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the `agx import` command.
pub fn run(args: ImportArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let manifest_path = cwd.join(MANIFEST_FILE);
    let manifest_exists = manifest_path.exists();

    // 1. Scan
    let scan = scan_project(&cwd);

    if !scan.has_detections() {
        println!("No existing agent configuration detected.");
        return Ok(());
    }

    // Filter to requested backends if --backend was specified.
    let backend_filter: Option<Vec<String>> = if args.backend.is_empty() {
        None
    } else {
        Some(args.backend.clone())
    };

    // 2. Print scan results
    print_import_summary(&scan, &backend_filter, manifest_exists, args.overwrite);

    // Warn if project appears to be already managed by agx.
    if is_agx_managed(&scan) {
        println!(
            "{}",
            style("Warning: this project appears to already be managed by agx.")
                .yellow()
                .bold()
        );
        println!(
            "  Importing would re-import agx's generated output. Review the result carefully."
        );
        println!();
    }

    if args.dry_run {
        println!("{}", style("Dry run — no files written.").dim());
        return Ok(());
    }

    // 3. Confirm
    if !args.non_interactive {
        let prompt = if manifest_exists {
            "Merge detected configuration into existing agent-environment.yml?"
        } else {
            "Create agent-environment.yml from detected configuration?"
        };
        let proceed = Confirm::new()
            .with_prompt(prompt)
            .default(true)
            .interact()
            .context("confirmation prompt")?;
        if !proceed {
            println!("Aborted. No files written.");
            return Ok(());
        }
    }

    // 4. Build manifest from scan
    let imported = build_manifest_from_scan(&cwd, &scan, &backend_filter)?;

    // 5. Merge or create
    let final_manifest = if manifest_exists {
        let existing = agx::manifest::load(&manifest_path)?;
        merge_manifests(&existing, &imported, args.overwrite)
    } else {
        imported
    };

    // 6. Write manifest
    let content = serde_yaml::to_string(&final_manifest).context("serializing manifest")?;
    fs::write(&manifest_path, &content)
        .with_context(|| format!("writing {}", manifest_path.display()))?;
    println!(
        "{} {}",
        style(if manifest_exists { "Updated:" } else { "Created:" })
            .green()
            .bold(),
        MANIFEST_FILE
    );

    // 7. Handle context files (CLAUDE.md → CONTEXT.md + overlay)
    let mut created_files = Vec::new();
    if scan.claude_md.is_some() && !manifest_exists {
        let convert = args.non_interactive
            || Confirm::new()
                .with_prompt("Convert CLAUDE.md to shared CONTEXT.md + overlay?")
                .default(true)
                .interact()
                .context("context conversion prompt")?;

        if convert {
            if let Some(ref claude_md_path) = scan.claude_md {
                let md_content = fs::read_to_string(claude_md_path)
                    .with_context(|| format!("reading {}", claude_md_path.display()))?;
                let (base, overlay) = split_claude_md(&md_content);

                let base_path = cwd.join("CONTEXT.md");
                fs::write(&base_path, &base).context("writing CONTEXT.md")?;
                println!("{} CONTEXT.md", style("Created:").green().bold());
                created_files.push("CONTEXT.md".to_string());

                let overlay_path = cwd.join("CONTEXT.claude-code.md");
                fs::write(&overlay_path, &overlay).context("writing CONTEXT.claude-code.md")?;
                println!(
                    "{} CONTEXT.claude-code.md",
                    style("Created:").green().bold()
                );
                created_files.push("CONTEXT.claude-code.md".to_string());
            }
        }
    }

    // 8. Update .gitignore
    let detected = scan.detected_backends();
    let backend_refs: Vec<&str> = detected.iter().copied().collect();
    let has_context = final_manifest.context.is_some();
    update_gitignore(&cwd, &backend_refs, has_context)?;

    // 9. Run install (unless --no-install)
    if !args.no_install {
        println!();
        install::run_install_at(&manifest_path)?;
    }

    // 10. Print next steps
    println!();
    println!(
        "{}",
        style("Review the generated manifest carefully:").yellow().bold()
    );
    println!("  - Settings were extracted into structured fields where possible");
    println!("  - Remaining settings are in the escape hatch (settings:)");
    println!("  - Local assets are referenced by path");
    if !created_files.is_empty() {
        println!("  - Context files were split heuristically — verify the base vs. overlay separation");
    }

    println!();
    println!("Next steps:");
    println!("  agx check       Validate the manifest");
    println!("  agx run         Launch in the managed environment");

    Ok(())
}

// ===========================================================================
// Manifest construction from scan results
// ===========================================================================

/// Build an `AgentEnvironment` manifest from scan results.
///
/// Extracts structured fields (model, isolation) from backend settings
/// instead of dumping raw JSON into the escape hatch.
pub fn build_manifest_from_scan(
    cwd: &Path,
    scan: &ScanResult,
    backend_filter: &Option<Vec<String>>,
) -> Result<AgentEnvironment> {
    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-agent-project")
        .to_string();

    let detected = scan.detected_backends();
    let active_backends: Vec<&str> = if let Some(filter) = backend_filter {
        detected
            .into_iter()
            .filter(|b| filter.iter().any(|f| f == b))
            .collect()
    } else {
        detected
    };

    let mut backends = BTreeMap::new();
    let mut settings = BTreeMap::new();
    let mut agents = BTreeMap::new();
    let mut skills = BTreeMap::new();
    let mcp_servers;

    // --- Claude Code ---
    if active_backends.contains(&"claude-code") {
        let (backend_config, claude_settings) =
            extract_claude_backend(&scan.claude_settings_json);
        backends.insert("claude-code".to_string(), backend_config);
        if let Some(s) = claude_settings {
            settings.insert("claude-code".to_string(), s);
        }

        // Agents
        for agent_name in &scan.claude_agents {
            let path = format!("./.claude/agents/{agent_name}.md");
            let (model, parameters) = extract_agent_frontmatter(cwd, &path);
            agents.insert(
                agent_name.clone(),
                AgentDependency {
                    git: None,
                    path,
                    tag: None,
                    branch: None,
                    commit: None,
                    model,
                    parameters,
                },
            );
        }

        // Skills
        for skill_name in &scan.claude_skills {
            skills.insert(
                skill_name.clone(),
                SkillDependency {
                    git: None,
                    path: format!("./.claude/skills/{skill_name}"),
                    tag: None,
                    branch: None,
                    commit: None,
                },
            );
        }
    }

    // --- Gemini CLI ---
    if active_backends.contains(&"gemini-cli") {
        let (backend_config, gemini_settings) =
            extract_gemini_backend(&scan.gemini_settings_json);
        backends.insert("gemini-cli".to_string(), backend_config);
        if let Some(s) = gemini_settings {
            settings.insert("gemini-cli".to_string(), s);
        }

        for agent_name in &scan.gemini_agents {
            let path = format!("./.gemini/agents/{agent_name}.md");
            let (model, parameters) = extract_agent_frontmatter(cwd, &path);
            let key = if agents.contains_key(agent_name) {
                format!("{agent_name}-gemini")
            } else {
                agent_name.clone()
            };
            agents.insert(
                key,
                AgentDependency {
                    git: None,
                    path,
                    tag: None,
                    branch: None,
                    commit: None,
                    model,
                    parameters,
                },
            );
        }

        for skill_name in &scan.gemini_skills {
            let key = if skills.contains_key(skill_name) {
                format!("{skill_name}-gemini")
            } else {
                skill_name.clone()
            };
            skills.insert(
                key,
                SkillDependency {
                    git: None,
                    path: format!("./.gemini/skills/{skill_name}"),
                    tag: None,
                    branch: None,
                    commit: None,
                },
            );
        }
    }

    // --- Cursor ---
    if active_backends.contains(&"cursor") {
        backends.insert(
            "cursor".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );

        for rule_name in &scan.cursor_rules {
            let key = if skills.contains_key(rule_name) {
                format!("{rule_name}-cursor")
            } else {
                rule_name.clone()
            };
            skills.insert(
                key,
                SkillDependency {
                    git: None,
                    path: format!("./.cursor/rules/{rule_name}.md"),
                    tag: None,
                    branch: None,
                    commit: None,
                },
            );
        }
    }

    // --- Codex ---
    if active_backends.contains(&"codex") {
        backends.insert(
            "codex".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );

        for skill_name in &scan.codex_skills {
            let key = if skills.contains_key(skill_name) {
                format!("{skill_name}-codex")
            } else {
                skill_name.clone()
            };
            skills.insert(
                key,
                SkillDependency {
                    git: None,
                    path: format!("./.codex/skills/{skill_name}"),
                    tag: None,
                    branch: None,
                    commit: None,
                },
            );
        }
    }

    // --- Windsurf ---
    if active_backends.contains(&"windsurf") {
        backends.insert(
            "windsurf".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );

        for rule_name in &scan.windsurf_rules {
            let key = if skills.contains_key(rule_name) {
                format!("{rule_name}-windsurf")
            } else {
                rule_name.clone()
            };
            skills.insert(
                key,
                SkillDependency {
                    git: None,
                    path: format!("./.windsurf/rules/{rule_name}.md"),
                    tag: None,
                    branch: None,
                    commit: None,
                },
            );
        }
    }

    // Set the first backend as default.
    if let Some(first) = backends.values_mut().next() {
        first.default = Some(true);
    }

    // MCP servers
    mcp_servers = if active_backends.contains(&"claude-code") {
        scan.mcp_servers.clone()
    } else {
        BTreeMap::new()
    };

    // Context config
    let context = if !active_backends.is_empty() {
        let mut overlays = BTreeMap::new();
        for &b in &active_backends {
            overlays.insert(b.to_string(), format!("./CONTEXT.{b}.md"));
        }
        Some(ContextConfig {
            base: "./CONTEXT.md".to_string(),
            overlays,
            children: vec![],
        })
    } else {
        None
    };

    Ok(AgentEnvironment {
        name: dir_name,
        version: Some("0.1.0".to_string()),
        description: None,
        backends,
        registries: BTreeMap::new(),
        plugins: BTreeMap::new(),
        skills,
        agents,
        mcp_servers,
        settings,
        paths: None,
        context,
        hooks: None,
        env_file: None,
    })
}

// ===========================================================================
// Settings extraction
// ===========================================================================

/// Extract structured `BackendConfig` and remaining settings from Claude Code's settings.json.
///
/// Pulls `model` into `BackendConfig.model` and filters out agx-generated markers.
/// Returns (backend_config, remaining_settings_as_yaml_value).
fn extract_claude_backend(
    json: &Option<serde_json::Value>,
) -> (BackendConfig, Option<serde_yaml::Value>) {
    let mut model_spec: Option<ModelSpec> = None;
    let mut remaining = serde_json::Map::new();

    if let Some(serde_json::Value::Object(obj)) = json {
        for (key, value) in obj {
            match key.as_str() {
                // Skip agx-generated markers.
                "_generated_by" => continue,
                // Extract model into structured field.
                "model" => {
                    if let Some(m) = value.as_str() {
                        model_spec = Some(ModelSpec::Simple(m.to_string()));
                    }
                }
                // Extract effort + plan model into full model config if present.
                "effort" | "planModel" => {
                    // These will be handled in a second pass below.
                    remaining.insert(key.clone(), value.clone());
                }
                // Filter out AGX_PROJECT from env.
                "env" => {
                    if let Some(env_obj) = value.as_object() {
                        let mut filtered = serde_json::Map::new();
                        for (k, v) in env_obj {
                            if k != "AGX_PROJECT" {
                                filtered.insert(k.clone(), v.clone());
                            }
                        }
                        if !filtered.is_empty() {
                            remaining
                                .insert(key.clone(), serde_json::Value::Object(filtered));
                        }
                    }
                }
                _ => {
                    remaining.insert(key.clone(), value.clone());
                }
            }
        }
    }

    // Promote model to Full config if effort or planModel are present.
    if remaining.contains_key("effort") || remaining.contains_key("planModel") {
        let base_id = match &model_spec {
            Some(ModelSpec::Simple(s)) => s.clone(),
            _ => String::new(),
        };
        if !base_id.is_empty() {
            let effort = remaining
                .remove("effort")
                .and_then(|v| v.as_str().map(String::from));
            let plan_model = remaining
                .remove("planModel")
                .and_then(|v| v.as_str().map(String::from));
            model_spec = Some(ModelSpec::Full(ModelConfig {
                id: base_id,
                extended_context: None,
                effort,
                plan_model,
                available: vec![],
                parameters: BTreeMap::new(),
            }));
        }
    }

    let remaining_yaml = if remaining.is_empty() {
        None
    } else {
        let json_str = serde_json::to_string(&serde_json::Value::Object(remaining))
            .unwrap_or_default();
        serde_yaml::from_str::<serde_yaml::Value>(&json_str).ok()
    };

    (
        BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: model_spec,
        },
        remaining_yaml,
    )
}

/// Extract structured `BackendConfig` and remaining settings from Gemini CLI's settings.json.
///
/// Pulls `model.name` into `BackendConfig.model` and extracts parameters from `modelConfigs`.
fn extract_gemini_backend(
    json: &Option<serde_json::Value>,
) -> (BackendConfig, Option<serde_yaml::Value>) {
    let mut model_spec: Option<ModelSpec> = None;
    let mut remaining = serde_json::Map::new();

    if let Some(serde_json::Value::Object(obj)) = json {
        for (key, value) in obj {
            match key.as_str() {
                "_generated_by" => continue,
                "model" => {
                    // Gemini uses { "model": { "name": "..." } } or { "model": "..." }
                    if let Some(m) = value.as_str() {
                        model_spec = Some(ModelSpec::Simple(m.to_string()));
                    } else if let Some(obj) = value.as_object() {
                        if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                            model_spec = Some(ModelSpec::Simple(name.to_string()));
                        }
                    }
                }
                "modelConfigs" => {
                    // Extract temperature, thinking_budget, etc. into parameters.
                    // These map to ModelConfig.parameters in the manifest.
                    if model_spec.is_none() {
                        remaining.insert(key.clone(), value.clone());
                    } else if let Some(configs) = value.as_object() {
                        let mut params = BTreeMap::new();
                        for (pk, pv) in configs {
                            let yaml_str =
                                serde_json::to_string(pv).unwrap_or_default();
                            if let Ok(yaml_val) =
                                serde_yaml::from_str::<serde_yaml::Value>(&yaml_str)
                            {
                                params.insert(pk.clone(), yaml_val);
                            }
                        }
                        if !params.is_empty() {
                            // Upgrade model_spec to Full if we have parameters.
                            let base_id = match &model_spec {
                                Some(ModelSpec::Simple(s)) => s.clone(),
                                _ => String::new(),
                            };
                            model_spec = Some(ModelSpec::Full(ModelConfig {
                                id: base_id,
                                extended_context: None,
                                effort: None,
                                plan_model: None,
                                available: vec![],
                                parameters: params,
                            }));
                        }
                    }
                }
                _ => {
                    remaining.insert(key.clone(), value.clone());
                }
            }
        }
    }

    let remaining_yaml = if remaining.is_empty() {
        None
    } else {
        let json_str = serde_json::to_string(&serde_json::Value::Object(remaining))
            .unwrap_or_default();
        serde_yaml::from_str::<serde_yaml::Value>(&json_str).ok()
    };

    (
        BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: model_spec,
        },
        remaining_yaml,
    )
}

/// Extract model and parameters from an agent's YAML frontmatter.
///
/// Reads the agent Markdown file and parses its frontmatter to pull out
/// `model` and any other YAML keys as parameters.
fn extract_agent_frontmatter(
    cwd: &Path,
    relative_path: &str,
) -> (Option<String>, BTreeMap<String, serde_yaml::Value>) {
    let path = cwd.join(relative_path.trim_start_matches("./"));
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return (None, BTreeMap::new()),
    };

    let (frontmatter, _body) = agx::backends::parse_frontmatter(&content);
    let fm_str = match frontmatter {
        Some(s) => s,
        None => return (None, BTreeMap::new()),
    };

    let fm_map: BTreeMap<String, serde_yaml::Value> = match serde_yaml::from_str(&fm_str) {
        Ok(m) => m,
        Err(_) => return (None, BTreeMap::new()),
    };

    let model = fm_map
        .get("model")
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut parameters = BTreeMap::new();
    for (key, value) in &fm_map {
        // Skip known non-parameter keys.
        if key == "model" || key == "description" || key == "name" {
            continue;
        }
        parameters.insert(key.clone(), value.clone());
    }

    (model, parameters)
}

// ===========================================================================
// Manifest merging
// ===========================================================================

/// Merge an imported manifest into an existing one.
///
/// - `name`, `version`, `description`: keep existing.
/// - `backends`: add new, keep existing unless `overwrite`.
/// - `mcp_servers`, `agents`, `skills`: add new, skip existing unless `overwrite`.
/// - `settings`: per-backend deep merge.
pub fn merge_manifests(
    existing: &AgentEnvironment,
    imported: &AgentEnvironment,
    overwrite: bool,
) -> AgentEnvironment {
    let mut result = existing.clone();

    // Backends
    for (key, value) in &imported.backends {
        if overwrite || !result.backends.contains_key(key) {
            result.backends.insert(key.clone(), value.clone());
        }
    }

    // MCP servers
    for (key, value) in &imported.mcp_servers {
        if overwrite || !result.mcp_servers.contains_key(key) {
            result.mcp_servers.insert(key.clone(), value.clone());
        }
    }

    // Agents
    for (key, value) in &imported.agents {
        if overwrite || !result.agents.contains_key(key) {
            result.agents.insert(key.clone(), value.clone());
        }
    }

    // Skills
    for (key, value) in &imported.skills {
        if overwrite || !result.skills.contains_key(key) {
            result.skills.insert(key.clone(), value.clone());
        }
    }

    // Settings (per-backend merge)
    for (backend, value) in &imported.settings {
        if overwrite || !result.settings.contains_key(backend) {
            result.settings.insert(backend.clone(), value.clone());
        }
    }

    // Context: keep existing if present, otherwise use imported.
    if result.context.is_none() {
        result.context = imported.context.clone();
    }

    result
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Check if the project appears to already be managed by agx.
fn is_agx_managed(scan: &ScanResult) -> bool {
    if let Some(ref json) = scan.claude_settings_json {
        if json.get("_generated_by").and_then(|v| v.as_str()).map_or(false, |s| s.contains("agx"))
        {
            return true;
        }
    }
    if let Some(ref json) = scan.gemini_settings_json {
        if json.get("_generated_by").and_then(|v| v.as_str()).map_or(false, |s| s.contains("agx"))
        {
            return true;
        }
    }
    false
}

/// Print a summary of what will be imported.
fn print_import_summary(
    scan: &ScanResult,
    backend_filter: &Option<Vec<String>>,
    manifest_exists: bool,
    overwrite: bool,
) {
    // Reuse the existing print_scan_results from init for the detection display.
    super::init::print_scan_results(scan);

    if let Some(filter) = backend_filter {
        println!(
            "  {} {}",
            style("Filtering to:").dim(),
            filter.join(", ")
        );
        println!();
    }

    if manifest_exists {
        if overwrite {
            println!(
                "  {} existing entries will be overwritten.",
                style("Merge mode:").yellow().bold()
            );
        } else {
            println!(
                "  {} existing entries will be preserved.",
                style("Merge mode:").yellow().bold()
            );
        }
        println!();
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_claude_project(dir: &Path) {
        fs::create_dir_all(dir.join(".claude/agents")).unwrap();
        fs::create_dir_all(dir.join(".claude/skills/deploy-check")).unwrap();
        fs::write(
            dir.join(".claude/settings.json"),
            r#"{"model": "opus", "permissions": {"allow": ["Bash"]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/agents/reviewer.md"),
            "---\nmodel: sonnet\n---\nYou are a code reviewer.\n",
        )
        .unwrap();
        fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers": {"playwright": {"url": "https://mcp.playwright.dev/sse"}}}"#,
        )
        .unwrap();
    }

    #[test]
    fn build_manifest_claude_only() {
        let dir = TempDir::new().unwrap();
        setup_claude_project(dir.path());

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        assert!(manifest.backends.contains_key("claude-code"));
        assert_eq!(manifest.backends["claude-code"].default, Some(true));
        assert!(manifest.agents.contains_key("reviewer"));
        assert_eq!(
            manifest.agents["reviewer"].model.as_deref(),
            Some("sonnet")
        );
        assert!(manifest.skills.contains_key("deploy-check"));
        assert!(manifest.mcp_servers.contains_key("playwright"));
    }

    #[test]
    fn build_manifest_extracts_model() {
        let dir = TempDir::new().unwrap();
        setup_claude_project(dir.path());

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        match &manifest.backends["claude-code"].model {
            Some(ModelSpec::Simple(s)) => assert_eq!(s, "opus"),
            other => panic!("expected Simple model, got {:?}", other),
        }
    }

    #[test]
    fn build_manifest_filters_generated_marker() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"_generated_by": "agx — DO NOT EDIT", "model": "sonnet"}"#,
        )
        .unwrap();

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        // _generated_by should not appear in remaining settings.
        if let Some(settings) = manifest.settings.get("claude-code") {
            let s = serde_yaml::to_string(settings).unwrap();
            assert!(
                !s.contains("_generated_by"),
                "settings should not contain _generated_by: {s}"
            );
        }
    }

    #[test]
    fn build_manifest_filters_agx_project_env() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"env": {"AGX_PROJECT": "true", "MY_VAR": "hello"}}"#,
        )
        .unwrap();

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        if let Some(settings) = manifest.settings.get("claude-code") {
            let s = serde_yaml::to_string(settings).unwrap();
            assert!(!s.contains("AGX_PROJECT"), "AGX_PROJECT should be filtered");
            assert!(s.contains("MY_VAR"), "MY_VAR should be preserved");
        }
    }

    #[test]
    fn build_manifest_multi_backend() {
        let dir = TempDir::new().unwrap();
        setup_claude_project(dir.path());

        // Add Gemini config.
        fs::create_dir_all(dir.path().join(".gemini/agents")).unwrap();
        fs::write(
            dir.path().join(".gemini/settings.json"),
            r#"{"model": {"name": "gemini-2.5-pro"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join(".gemini/agents/helper.md"),
            "You are a helper.\n",
        )
        .unwrap();

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        assert!(manifest.backends.contains_key("claude-code"));
        assert!(manifest.backends.contains_key("gemini-cli"));
        assert!(manifest.agents.contains_key("reviewer"));
        assert!(manifest.agents.contains_key("helper"));

        match &manifest.backends["gemini-cli"].model {
            Some(ModelSpec::Simple(s)) => assert_eq!(s, "gemini-2.5-pro"),
            other => panic!("expected Simple model, got {:?}", other),
        }
    }

    #[test]
    fn build_manifest_backend_filter() {
        let dir = TempDir::new().unwrap();
        setup_claude_project(dir.path());
        fs::create_dir_all(dir.path().join(".gemini")).unwrap();

        let scan = scan_project(dir.path());
        let filter = Some(vec!["claude-code".to_string()]);
        let manifest = build_manifest_from_scan(dir.path(), &scan, &filter).unwrap();

        assert!(manifest.backends.contains_key("claude-code"));
        assert!(!manifest.backends.contains_key("gemini-cli"));
    }

    #[test]
    fn merge_manifests_preserves_existing() {
        let existing = AgentEnvironment {
            name: "my-project".to_string(),
            version: Some("1.0.0".to_string()),
            description: Some("Existing project".to_string()),
            backends: {
                let mut m = BTreeMap::new();
                m.insert(
                    "claude-code".to_string(),
                    BackendConfig {
                        default: Some(true),
                        isolation: Some(IsolationMode::Permissive),
                        model: Some(ModelSpec::Simple("opus".to_string())),
                    },
                );
                m
            },
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: {
                let mut m = BTreeMap::new();
                m.insert(
                    "reviewer".to_string(),
                    AgentDependency {
                        git: None,
                        path: "./agents/reviewer.md".to_string(),
                        tag: None,
                        branch: None,
                        commit: None,
                        model: Some("opus".to_string()),
                        parameters: BTreeMap::new(),
                    },
                );
                m
            },
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
            env_file: None,
        };

        let imported = AgentEnvironment {
            name: "imported".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends: {
                let mut m = BTreeMap::new();
                m.insert(
                    "gemini-cli".to_string(),
                    BackendConfig {
                        default: None,
                        isolation: Some(IsolationMode::Strict),
                        model: None,
                    },
                );
                m
            },
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: {
                let mut m = BTreeMap::new();
                m.insert(
                    "reviewer".to_string(),
                    AgentDependency {
                        git: None,
                        path: "./.claude/agents/reviewer.md".to_string(),
                        tag: None,
                        branch: None,
                        commit: None,
                        model: Some("sonnet".to_string()),
                        parameters: BTreeMap::new(),
                    },
                );
                m.insert(
                    "helper".to_string(),
                    AgentDependency {
                        git: None,
                        path: "./.gemini/agents/helper.md".to_string(),
                        tag: None,
                        branch: None,
                        commit: None,
                        model: None,
                        parameters: BTreeMap::new(),
                    },
                );
                m
            },
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: Some(ContextConfig {
                base: "./CONTEXT.md".to_string(),
                overlays: BTreeMap::new(),
                children: vec![],
            }),
            hooks: None,
            env_file: None,
        };

        let merged = merge_manifests(&existing, &imported, false);

        // Name, version, description from existing.
        assert_eq!(merged.name, "my-project");
        assert_eq!(merged.version.as_deref(), Some("1.0.0"));
        assert_eq!(merged.description.as_deref(), Some("Existing project"));

        // Existing backend preserved (not overwritten).
        assert_eq!(
            merged.backends["claude-code"].isolation,
            Some(IsolationMode::Permissive)
        );

        // New backend added.
        assert!(merged.backends.contains_key("gemini-cli"));

        // Existing agent preserved (reviewer stays as opus, not overwritten to sonnet).
        assert_eq!(
            merged.agents["reviewer"].model.as_deref(),
            Some("opus")
        );
        assert_eq!(merged.agents["reviewer"].path, "./agents/reviewer.md");

        // New agent added.
        assert!(merged.agents.contains_key("helper"));

        // Context from imported since existing had none.
        assert!(merged.context.is_some());
    }

    #[test]
    fn merge_manifests_with_overwrite() {
        let existing = AgentEnvironment {
            name: "my-project".to_string(),
            version: Some("1.0.0".to_string()),
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: {
                let mut m = BTreeMap::new();
                m.insert(
                    "reviewer".to_string(),
                    AgentDependency {
                        git: None,
                        path: "./old-path.md".to_string(),
                        tag: None,
                        branch: None,
                        commit: None,
                        model: Some("opus".to_string()),
                        parameters: BTreeMap::new(),
                    },
                );
                m
            },
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
            env_file: None,
        };

        let imported = AgentEnvironment {
            name: "imported".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: {
                let mut m = BTreeMap::new();
                m.insert(
                    "reviewer".to_string(),
                    AgentDependency {
                        git: None,
                        path: "./.claude/agents/reviewer.md".to_string(),
                        tag: None,
                        branch: None,
                        commit: None,
                        model: Some("sonnet".to_string()),
                        parameters: BTreeMap::new(),
                    },
                );
                m
            },
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
            env_file: None,
        };

        let merged = merge_manifests(&existing, &imported, true);

        // With overwrite, imported values win.
        assert_eq!(
            merged.agents["reviewer"].model.as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            merged.agents["reviewer"].path,
            "./.claude/agents/reviewer.md"
        );
    }

    #[test]
    fn is_agx_managed_detects_marker() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: true,
            claude_settings_value: None,
            claude_settings_json: Some(serde_json::json!({
                "_generated_by": "agx — DO NOT EDIT. Update agent-environment.yml and run: agx generate",
                "model": "sonnet"
            })),
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: vec![],
            claude_skills: vec![],
            gemini_dir: false,
            gemini_settings: false,
            gemini_settings_json: None,
            gemini_settings_value: None,
            gemini_agents: vec![],
            gemini_skills: vec![],
            cursor_dir: false,
            cursor_rules: vec![],
            codex_dir: false,
            codex_skills: vec![],
            windsurf_dir: false,
            windsurf_rules: vec![],
        };

        assert!(is_agx_managed(&scan));
    }

    #[test]
    fn is_agx_managed_negative() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: true,
            claude_settings_value: None,
            claude_settings_json: Some(serde_json::json!({"model": "sonnet"})),
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: vec![],
            claude_skills: vec![],
            gemini_dir: false,
            gemini_settings: false,
            gemini_settings_json: None,
            gemini_settings_value: None,
            gemini_agents: vec![],
            gemini_skills: vec![],
            cursor_dir: false,
            cursor_rules: vec![],
            codex_dir: false,
            codex_skills: vec![],
            windsurf_dir: false,
            windsurf_rules: vec![],
        };

        assert!(!is_agx_managed(&scan));
    }

    #[test]
    fn extract_claude_backend_full_model() {
        let json = Some(serde_json::json!({
            "model": "opus",
            "effort": "high",
            "planModel": "sonnet",
            "permissions": {"allow": ["Bash"]}
        }));

        let (config, remaining) = extract_claude_backend(&json);

        match &config.model {
            Some(ModelSpec::Full(mc)) => {
                assert_eq!(mc.id, "opus");
                assert_eq!(mc.effort.as_deref(), Some("high"));
                assert_eq!(mc.plan_model.as_deref(), Some("sonnet"));
            }
            other => panic!("expected Full model, got {:?}", other),
        }

        // permissions should be in remaining settings.
        assert!(remaining.is_some());
        let s = serde_yaml::to_string(&remaining.unwrap()).unwrap();
        assert!(s.contains("permissions"));
        // effort and planModel should NOT be in remaining.
        assert!(!s.contains("effort"));
        assert!(!s.contains("planModel"));
    }

    #[test]
    fn extract_gemini_backend_with_model_name() {
        let json = Some(serde_json::json!({
            "model": {"name": "gemini-2.5-pro"},
            "customInstructions": "Be helpful"
        }));

        let (config, remaining) = extract_gemini_backend(&json);

        match &config.model {
            Some(ModelSpec::Simple(s)) => assert_eq!(s, "gemini-2.5-pro"),
            other => panic!("expected Simple model, got {:?}", other),
        }

        assert!(remaining.is_some());
        let s = serde_yaml::to_string(&remaining.unwrap()).unwrap();
        assert!(s.contains("customInstructions"));
    }

    #[test]
    fn extract_agent_frontmatter_with_model() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::write(
            dir.path().join(".claude/agents/reviewer.md"),
            "---\nmodel: sonnet\ndescription: Reviews code\n---\nYou review code.\n",
        )
        .unwrap();

        let (model, params) =
            extract_agent_frontmatter(dir.path(), "./.claude/agents/reviewer.md");

        assert_eq!(model.as_deref(), Some("sonnet"));
        // description is a known non-parameter key, so it should not be in params.
        assert!(!params.contains_key("description"));
        assert!(!params.contains_key("model"));
    }

    #[test]
    fn extract_agent_frontmatter_no_frontmatter() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::write(
            dir.path().join(".claude/agents/simple.md"),
            "You are a simple agent.\n",
        )
        .unwrap();

        let (model, params) =
            extract_agent_frontmatter(dir.path(), "./.claude/agents/simple.md");

        assert!(model.is_none());
        assert!(params.is_empty());
    }

    #[test]
    fn cursor_rules_imported_as_skills() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".cursor/rules")).unwrap();
        fs::write(
            dir.path().join(".cursor/rules/style-guide.md"),
            "Follow this style guide.\n",
        )
        .unwrap();

        let scan = scan_project(dir.path());
        let manifest = build_manifest_from_scan(dir.path(), &scan, &None).unwrap();

        assert!(manifest.backends.contains_key("cursor"));
        assert!(manifest.skills.contains_key("style-guide"));
        assert_eq!(
            manifest.skills["style-guide"].path,
            "./.cursor/rules/style-guide.md"
        );
    }
}
