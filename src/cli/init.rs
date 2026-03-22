//! `agx init` — scaffold a new project or import existing configuration.
//!
//! Two modes:
//! 1. **New project** — interactive wizard collects project name, backends,
//!    isolation mode, context files, and registries, then generates a manifest.
//! 2. **Existing project** — scans for agent configuration files (`.claude/`,
//!    `.mcp.json`, `CLAUDE.md`, etc.), displays what was found, and offers to
//!    import the detected configuration into an `agent-environment.yml`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use agx::config::MANIFEST_FILE;
use agx::manifest::types::{
    AgentDependency, AgentEnvironment, BackendConfig, ContextConfig, IsolationMode, McpServerConfig,
    RegistryConfig, SkillDependency,
};

use super::install;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Scaffold a blank project, ignoring existing config
    #[arg(long)]
    pub force: bool,

    /// Scan and print detected config without creating files
    #[arg(long)]
    pub scan_only: bool,

    /// Non-interactive mode: accept defaults (project name from directory,
    /// claude-code backend, strict isolation, context files created)
    #[arg(short = 'y', long = "yes")]
    pub non_interactive: bool,

    /// Project name (used with --yes; defaults to directory name)
    #[arg(long)]
    pub name: Option<String>,

    /// Backends to enable (used with --yes; defaults to claude-code)
    #[arg(long, value_delimiter = ',')]
    pub backend: Vec<String>,
}

// ---------------------------------------------------------------------------
// Known backends presented in the wizard
// ---------------------------------------------------------------------------

const KNOWN_BACKENDS: &[&str] = &["claude-code", "gemini-cli", "codex", "cursor", "windsurf"];

// ---------------------------------------------------------------------------
// Keywords that mark a CLAUDE.md section as Claude-specific (for splitting)
// ---------------------------------------------------------------------------

const CLAUDE_KEYWORDS: &[&str] = &[
    "/compact",
    "/clear",
    "/init",
    "/login",
    "/logout",
    "/cost",
    "/doctor",
    "/memory",
    "/permissions",
    "/mcp",
    "/config",
    "/review",
    "@mention",
    "Claude memory",
    "claude memory",
    "MCP server",
    "mcp server",
    "MCP tool",
    "mcp tool",
    "claude code",
    "Claude Code",
    "CLAUDE.md",
    "claude.md",
    "TodoRead",
    "TodoWrite",
    "Bash(",
    "Read(",
    "Edit(",
    "Write(",
    "Glob(",
    "Grep(",
    ".claude/",
    "slash command",
    "extended thinking",
];

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the `agx init` command.
pub fn run(args: InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;

    // If manifest already exists and neither --force nor --scan-only, warn.
    let manifest_path = cwd.join(MANIFEST_FILE);
    if manifest_path.exists() && !args.force && !args.scan_only {
        anyhow::bail!(
            "{} already exists. Use --force to overwrite.",
            MANIFEST_FILE
        );
    }

    // Scan for existing agent config files.
    let scan = scan_project(&cwd);

    if args.scan_only {
        print_scan_results(&scan);
        return Ok(());
    }

    if args.non_interactive {
        // Non-interactive mode: use defaults or CLI flags.
        run_non_interactive_flow(&cwd, &args)
    } else if !args.force && scan.has_detections() {
        // Existing-project flow.
        print_scan_results(&scan);
        run_import_flow(&cwd, &scan)
    } else {
        // New-project flow (interactive wizard).
        run_new_project_flow(&cwd)
    }
}

// ===========================================================================
// Non-interactive mode
// ===========================================================================

/// Non-interactive init: uses defaults or CLI flags.
///
/// Defaults: project name from directory, claude-code backend, strict isolation,
/// context files created. Override with `--name`, `--backend`.
fn run_non_interactive_flow(cwd: &Path, args: &InitArgs) -> Result<()> {
    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-agent-project")
        .to_string();

    let name = args.name.clone().unwrap_or(dir_name);

    let selected_backends: Vec<String> = if args.backend.is_empty() {
        vec!["claude-code".to_string()]
    } else {
        args.backend.clone()
    };

    let mut backends = BTreeMap::new();
    for (i, backend) in selected_backends.iter().enumerate() {
        backends.insert(
            backend.clone(),
            BackendConfig {
                default: if i == 0 { Some(true) } else { None },
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
    }

    let mut overlays = BTreeMap::new();
    for b in &selected_backends {
        overlays.insert(b.clone(), format!("./CONTEXT.{b}.md"));
    }
    let context = Some(ContextConfig {
        base: "./CONTEXT.md".to_string(),
        overlays,
        children: vec![],
    });

    let manifest = AgentEnvironment {
        name,
        version: Some("0.1.0".to_string()),
        description: None,
        backends,
        registries: BTreeMap::new(),
        plugins: BTreeMap::new(),
        skills: BTreeMap::new(),
        agents: BTreeMap::new(),
        mcp_servers: BTreeMap::new(),
        settings: BTreeMap::new(),
        paths: None,
        context,
        hooks: None,
        env_file: None,
    };

    let mut created_files = Vec::new();

    write_manifest(cwd, &manifest)?;
    created_files.push(MANIFEST_FILE.to_string());

    write_context_base(cwd)?;
    created_files.push("CONTEXT.md".to_string());

    for b in &selected_backends {
        let filename = format!("CONTEXT.{b}.md");
        write_context_overlay(cwd, b, &filename)?;
        created_files.push(filename);
    }

    let backend_refs: Vec<&str> = selected_backends.iter().map(|s| s.as_str()).collect();
    update_gitignore(cwd, &backend_refs, true)?;
    created_files.push(".gitignore".to_string());

    for f in &created_files {
        println!(
            "{} {}",
            style("Created:").green().bold(),
            f
        );
    }

    // Run install to generate backend configs immediately.
    println!();
    let manifest_path = cwd.join(MANIFEST_FILE);
    install::run_install_at(&manifest_path)?;

    println!();
    println!("Next steps:");
    println!("  Edit {} to add dependencies", MANIFEST_FILE);
    println!("  agx run        Launch the default backend");

    Ok(())
}

// ===========================================================================
// New-project interactive wizard
// ===========================================================================

/// Interactive wizard for scaffolding a brand-new project.
fn run_new_project_flow(cwd: &Path) -> Result<()> {
    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-agent-project")
        .to_string();

    // 1. Project name
    let name: String = Input::new()
        .with_prompt("Project name")
        .default(dir_name)
        .interact_text()
        .context("project name prompt")?;

    // 2. Select backends
    let backend_selections = MultiSelect::new()
        .with_prompt("Select backends")
        .items(KNOWN_BACKENDS)
        .interact()
        .context("backend selection prompt")?;

    if backend_selections.is_empty() {
        anyhow::bail!("At least one backend must be selected.");
    }

    let selected_backends: Vec<&str> = backend_selections
        .iter()
        .map(|&i| KNOWN_BACKENDS[i])
        .collect();

    // 3. Isolation mode
    let isolation_options = &["strict", "permissive", "bare"];
    let isolation_idx = Select::new()
        .with_prompt("Default isolation mode")
        .items(isolation_options)
        .default(0)
        .interact()
        .context("isolation mode prompt")?;
    let isolation = match isolation_options[isolation_idx] {
        "strict" => IsolationMode::Strict,
        "bare" => IsolationMode::Bare,
        _ => IsolationMode::Permissive,
    };

    // 4. Create context files?
    let create_context = Confirm::new()
        .with_prompt("Create context files?")
        .default(true)
        .interact()
        .context("context files prompt")?;

    // 5. Registries (loop)
    let mut registries: BTreeMap<String, RegistryConfig> = BTreeMap::new();
    loop {
        let url: String = Input::new()
            .with_prompt("Add a marketplace registry? (enter URL or skip)")
            .default("skip".to_string())
            .interact_text()
            .context("registry URL prompt")?;

        if url == "skip" || url.is_empty() {
            break;
        }

        let reg_name: String = Input::new()
            .with_prompt("Name for this registry")
            .interact_text()
            .context("registry name prompt")?;

        registries.insert(reg_name, RegistryConfig { url, auth: None, pin: None });
    }

    // Build manifest struct.
    let mut backends = BTreeMap::new();
    for (i, &backend) in selected_backends.iter().enumerate() {
        backends.insert(
            backend.to_string(),
            BackendConfig {
                default: if i == 0 { Some(true) } else { None },
                isolation: Some(isolation.clone()),
                model: None,
            },
        );
    }

    let context = if create_context {
        let mut overlays = BTreeMap::new();
        for &b in &selected_backends {
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

    let manifest = AgentEnvironment {
        name,
        version: Some("0.1.0".to_string()),
        description: None,
        backends,
        registries,
        plugins: BTreeMap::new(),
        skills: BTreeMap::new(),
        agents: BTreeMap::new(),
        mcp_servers: BTreeMap::new(),
        settings: BTreeMap::new(),
        paths: None,
        context,
        hooks: None,
        env_file: None,
    };

    // Write files.
    let mut created_files = Vec::new();

    write_manifest(cwd, &manifest)?;
    created_files.push(MANIFEST_FILE.to_string());

    if create_context {
        write_context_base(cwd)?;
        created_files.push("CONTEXT.md".to_string());

        for &b in &selected_backends {
            let filename = format!("CONTEXT.{b}.md");
            write_context_overlay(cwd, b, &filename)?;
            created_files.push(filename);
        }
    }

    update_gitignore(cwd, &selected_backends, create_context)?;
    created_files.push(".gitignore".to_string());

    // Print summary.
    println!();
    for f in &created_files {
        println!(
            "{} {}",
            style("Created:").green().bold(),
            f
        );
    }

    // Run install to generate backend configs immediately.
    println!();
    let manifest_path = cwd.join(MANIFEST_FILE);
    install::run_install_at(&manifest_path)?;

    println!();
    println!("Next steps:");
    println!("  Edit {} to add dependencies", MANIFEST_FILE);
    println!("  agx run        Launch the default backend");

    Ok(())
}

// ===========================================================================
// Existing-project import flow
// ===========================================================================

/// Import detected configuration into a manifest.
fn run_import_flow(cwd: &Path, scan: &ScanResult) -> Result<()> {
    let import = Confirm::new()
        .with_prompt("Import detected configuration into agent-environment.yml?")
        .default(true)
        .interact()
        .context("import prompt")?;

    if !import {
        println!("Aborted. No files created.");
        return Ok(());
    }

    // Should we convert CLAUDE.md?
    let convert_claude = scan.claude_md.is_some()
        && Confirm::new()
            .with_prompt("Convert CLAUDE.md to shared CONTEXT.md + overlay?")
            .default(true)
            .interact()
            .context("context conversion prompt")?;

    // Build the manifest from scan results.
    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-agent-project")
        .to_string();

    let mut backends = BTreeMap::new();
    let detected_backends: Vec<&str> = scan.detected_backends();
    for (i, &b) in detected_backends.iter().enumerate() {
        backends.insert(
            b.to_string(),
            BackendConfig {
                default: if i == 0 { Some(true) } else { None },
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
    }

    // MCP servers from .mcp.json
    let mcp_servers = scan.mcp_servers.clone();

    // Settings from .claude/settings.json
    let mut settings = BTreeMap::new();
    if let Some(ref val) = scan.claude_settings_value {
        settings.insert("claude-code".to_string(), val.clone());
    }

    // Agents from .claude/agents/
    let mut agents = BTreeMap::new();
    for agent_name in &scan.claude_agents {
        agents.insert(
            agent_name.clone(),
            AgentDependency {
                git: None,
                path: format!("./.claude/agents/{agent_name}.md"),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
    }

    // Skills from .claude/skills/
    let mut skills = BTreeMap::new();
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

    // Context
    let has_context = convert_claude || !detected_backends.is_empty();
    let context = if has_context {
        let mut overlays = BTreeMap::new();
        for &b in &detected_backends {
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

    let manifest = AgentEnvironment {
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
    };

    // Write files.
    let mut created_files = Vec::new();

    write_manifest(cwd, &manifest)?;
    created_files.push(MANIFEST_FILE.to_string());

    if convert_claude {
        if let Some(ref claude_md_path) = scan.claude_md {
            let content = fs::read_to_string(claude_md_path)
                .with_context(|| format!("reading {}", claude_md_path.display()))?;
            let (base, overlay) = split_claude_md(&content);

            let base_path = cwd.join("CONTEXT.md");
            fs::write(&base_path, &base).context("writing CONTEXT.md")?;
            created_files.push("CONTEXT.md".to_string());

            let overlay_path = cwd.join("CONTEXT.claude-code.md");
            fs::write(&overlay_path, &overlay).context("writing CONTEXT.claude-code.md")?;
            created_files.push("CONTEXT.claude-code.md".to_string());
        }
    }

    let backend_names: Vec<&str> = detected_backends.clone();
    update_gitignore(cwd, &backend_names, has_context)?;
    created_files.push(".gitignore".to_string());

    // Print summary.
    println!();
    for f in &created_files {
        println!(
            "{} {}",
            style("Created:").green().bold(),
            f
        );
    }

    // Run install to generate backend configs immediately.
    println!();
    let manifest_path = cwd.join(MANIFEST_FILE);
    install::run_install_at(&manifest_path)?;

    println!();
    println!(
        "{}",
        style("Review the generated manifest carefully:").yellow().bold()
    );
    println!("  - MCP servers were imported as-is (check env var references)");
    println!("  - Local assets are referenced by path");
    if convert_claude {
        println!("  - Context files were split heuristically — verify the base vs. overlay separation");
    }

    println!();
    println!("Next steps:");
    println!("  agx check       Validate the generated manifest");
    println!("  agx run         Launch in the managed environment");

    Ok(())
}

// ===========================================================================
// Project scanner
// ===========================================================================

/// Results from scanning a project directory for existing agent configuration.
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    /// Path to CLAUDE.md if it exists.
    pub claude_md: Option<PathBuf>,
    /// Line count of CLAUDE.md.
    pub claude_md_lines: usize,

    /// Path to GEMINI.md if it exists.
    pub gemini_md: Option<PathBuf>,
    /// Line count of GEMINI.md.
    pub gemini_md_lines: usize,

    /// Path to AGENTS.md if it exists.
    pub agents_md: Option<PathBuf>,

    /// Whether .claude/settings.json exists.
    pub claude_settings: bool,
    /// Parsed value from .claude/settings.json (as serde_yaml::Value for inclusion in settings).
    pub claude_settings_value: Option<serde_yaml::Value>,
    /// Raw JSON value from .claude/settings.json (for structured extraction).
    pub claude_settings_json: Option<serde_json::Value>,

    /// Whether .mcp.json exists.
    pub mcp_json: bool,
    /// Parsed MCP servers from .mcp.json.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,

    /// Agent names found in .claude/agents/*.md.
    pub claude_agents: Vec<String>,

    /// Skill directory names found in .claude/skills/*/.
    pub claude_skills: Vec<String>,

    /// Whether .gemini/ directory exists.
    pub gemini_dir: bool,
    /// Whether .gemini/settings.json exists.
    pub gemini_settings: bool,
    /// Raw JSON value from .gemini/settings.json (for structured extraction).
    pub gemini_settings_json: Option<serde_json::Value>,
    /// Parsed value from .gemini/settings.json (as serde_yaml::Value for inclusion in settings).
    pub gemini_settings_value: Option<serde_yaml::Value>,
    /// Agent names found in .gemini/agents/*.md.
    pub gemini_agents: Vec<String>,
    /// Skill directory names found in .gemini/skills/*/.
    pub gemini_skills: Vec<String>,

    /// Whether .cursor/ directory exists.
    pub cursor_dir: bool,
    /// Rule names found in .cursor/rules/*.md (skill equivalents).
    pub cursor_rules: Vec<String>,

    /// Whether .codex/ directory exists.
    pub codex_dir: bool,
    /// Skill directory names found in .codex/skills/*/.
    pub codex_skills: Vec<String>,

    /// Whether .windsurf/ directory exists.
    pub windsurf_dir: bool,
    /// Rule names found in .windsurf/rules/*.md (skill equivalents).
    pub windsurf_rules: Vec<String>,
}

impl ScanResult {
    /// Returns true if any agent configuration was detected.
    pub fn has_detections(&self) -> bool {
        self.claude_md.is_some()
            || self.gemini_md.is_some()
            || self.agents_md.is_some()
            || self.claude_settings
            || self.mcp_json
            || !self.claude_agents.is_empty()
            || !self.claude_skills.is_empty()
            || self.gemini_dir
            || self.gemini_settings
            || !self.gemini_agents.is_empty()
            || !self.gemini_skills.is_empty()
            || self.cursor_dir
            || !self.cursor_rules.is_empty()
            || self.codex_dir
            || !self.codex_skills.is_empty()
            || self.windsurf_dir
            || !self.windsurf_rules.is_empty()
    }

    /// Return the list of detected backend names.
    pub fn detected_backends(&self) -> Vec<&str> {
        let mut backends = Vec::new();

        let has_claude = self.claude_md.is_some()
            || self.claude_settings
            || self.mcp_json
            || !self.claude_agents.is_empty()
            || !self.claude_skills.is_empty();

        if has_claude {
            backends.push("claude-code");
        }

        if self.gemini_md.is_some()
            || self.gemini_dir
            || self.gemini_settings
            || !self.gemini_agents.is_empty()
            || !self.gemini_skills.is_empty()
        {
            backends.push("gemini-cli");
        }

        if self.cursor_dir || !self.cursor_rules.is_empty() {
            backends.push("cursor");
        }

        if self.codex_dir || !self.codex_skills.is_empty() {
            backends.push("codex");
        }

        if self.windsurf_dir || !self.windsurf_rules.is_empty() {
            backends.push("windsurf");
        }

        backends
    }
}

/// Scan a project directory for existing agent configuration files.
pub fn scan_project(cwd: &Path) -> ScanResult {
    let mut result = ScanResult {
        claude_md: None,
        claude_md_lines: 0,
        gemini_md: None,
        gemini_md_lines: 0,
        agents_md: None,
        claude_settings: false,
        claude_settings_value: None,
        claude_settings_json: None,
        mcp_json: false,
        mcp_servers: BTreeMap::new(),
        claude_agents: Vec::new(),
        claude_skills: Vec::new(),
        gemini_dir: false,
        gemini_settings: false,
        gemini_settings_json: None,
        gemini_settings_value: None,
        gemini_agents: Vec::new(),
        gemini_skills: Vec::new(),
        cursor_dir: false,
        cursor_rules: Vec::new(),
        codex_dir: false,
        codex_skills: Vec::new(),
        windsurf_dir: false,
        windsurf_rules: Vec::new(),
    };

    // CLAUDE.md
    let claude_md_path = cwd.join("CLAUDE.md");
    if claude_md_path.exists() {
        if let Ok(content) = fs::read_to_string(&claude_md_path) {
            result.claude_md_lines = content.lines().count();
        }
        result.claude_md = Some(claude_md_path);
    }

    // GEMINI.md
    let gemini_md_path = cwd.join("GEMINI.md");
    if gemini_md_path.exists() {
        if let Ok(content) = fs::read_to_string(&gemini_md_path) {
            result.gemini_md_lines = content.lines().count();
        }
        result.gemini_md = Some(gemini_md_path);
    }

    // AGENTS.md
    let agents_md_path = cwd.join("AGENTS.md");
    if agents_md_path.exists() {
        result.agents_md = Some(agents_md_path);
    }

    // .claude/settings.json
    let claude_settings_path = cwd.join(".claude/settings.json");
    if claude_settings_path.exists() {
        result.claude_settings = true;
        if let Ok(content) = fs::read_to_string(&claude_settings_path) {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                result.claude_settings_json = Some(json_val.clone());
                // Convert JSON value to serde_yaml::Value for inclusion in the settings map.
                let yaml_str = serde_json::to_string(&json_val).unwrap_or_default();
                if let Ok(yaml_val) = serde_yaml::from_str::<serde_yaml::Value>(&yaml_str) {
                    result.claude_settings_value = Some(yaml_val);
                }
            }
        }
    }

    // .mcp.json
    let mcp_json_path = cwd.join(".mcp.json");
    if mcp_json_path.exists() {
        result.mcp_json = true;
        result.mcp_servers = parse_mcp_json(&mcp_json_path);
    }

    // .claude/agents/*.md
    let agents_dir = cwd.join(".claude/agents");
    if agents_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        result.claude_agents.push(stem.to_string());
                    }
                }
            }
            result.claude_agents.sort();
        }
    }

    // .claude/skills/*/
    let skills_dir = cwd.join(".claude/skills");
    if skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        result.claude_skills.push(name.to_string());
                    }
                }
            }
            result.claude_skills.sort();
        }
    }

    // .gemini/ directory
    let gemini_dir = cwd.join(".gemini");
    if gemini_dir.is_dir() {
        result.gemini_dir = true;
    }

    // .gemini/settings.json
    let gemini_settings_path = cwd.join(".gemini/settings.json");
    if gemini_settings_path.exists() {
        result.gemini_settings = true;
        if let Ok(content) = fs::read_to_string(&gemini_settings_path) {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                result.gemini_settings_json = Some(json_val.clone());
                let yaml_str = serde_json::to_string(&json_val).unwrap_or_default();
                if let Ok(yaml_val) = serde_yaml::from_str::<serde_yaml::Value>(&yaml_str) {
                    result.gemini_settings_value = Some(yaml_val);
                }
            }
        }
    }

    // .gemini/agents/*.md
    let gemini_agents_dir = cwd.join(".gemini/agents");
    if gemini_agents_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&gemini_agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        result.gemini_agents.push(stem.to_string());
                    }
                }
            }
            result.gemini_agents.sort();
        }
    }

    // .gemini/skills/*/
    let gemini_skills_dir = cwd.join(".gemini/skills");
    if gemini_skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&gemini_skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        result.gemini_skills.push(name.to_string());
                    }
                }
            }
            result.gemini_skills.sort();
        }
    }

    // .cursor/ directory and rules
    let cursor_dir = cwd.join(".cursor");
    if cursor_dir.is_dir() {
        result.cursor_dir = true;
    }
    let cursor_rules_dir = cwd.join(".cursor/rules");
    if cursor_rules_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&cursor_rules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md")
                    || path.extension().and_then(|e| e.to_str()) == Some("mdc")
                {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        result.cursor_rules.push(stem.to_string());
                    }
                }
            }
            result.cursor_rules.sort();
        }
    }

    // .codex/ directory and skills
    let codex_dir = cwd.join(".codex");
    if codex_dir.is_dir() {
        result.codex_dir = true;
    }
    let codex_skills_dir = cwd.join(".codex/skills");
    if codex_skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&codex_skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        result.codex_skills.push(name.to_string());
                    }
                }
            }
            result.codex_skills.sort();
        }
    }

    // .windsurf/ directory and rules
    let windsurf_dir = cwd.join(".windsurf");
    if windsurf_dir.is_dir() {
        result.windsurf_dir = true;
    }
    let windsurf_rules_dir = cwd.join(".windsurf/rules");
    if windsurf_rules_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&windsurf_rules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        result.windsurf_rules.push(stem.to_string());
                    }
                }
            }
            result.windsurf_rules.sort();
        }
    }

    result
}

/// Parse `.mcp.json` into a map of MCP server configs.
fn parse_mcp_json(path: &Path) -> BTreeMap<String, McpServerConfig> {
    let mut servers = BTreeMap::new();

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return servers,
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return servers,
    };

    // .mcp.json has the shape: { "mcpServers": { "name": { ... } } }
    let mcp_servers = json
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .or_else(|| json.as_object());

    if let Some(map) = mcp_servers {
        for (name, value) in map {
            let url = value.get("url").and_then(|v| v.as_str()).map(String::from);
            let command = value
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from);
            let args = value
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let env = value
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            servers.insert(
                name.clone(),
                McpServerConfig {
                    url,
                    command,
                    args,
                    env,
                },
            );
        }
    }

    servers
}

/// Pretty-print scan results to stdout.
pub fn print_scan_results(scan: &ScanResult) {
    println!(
        "{}",
        style("Scanning project for existing agent configuration...").bold()
    );
    println!();

    if !scan.has_detections() {
        println!("  No existing agent configuration detected.");
        return;
    }

    println!("{}", style("Detected:").bold());
    println!("  {}:", style("Backends").bold());

    // Claude Code
    let has_claude = scan.claude_md.is_some()
        || scan.claude_settings
        || scan.mcp_json
        || !scan.claude_agents.is_empty()
        || !scan.claude_skills.is_empty();

    if has_claude {
        println!("    {} {}", style("●").green(), style("claude-code").bold());

        if let Some(ref _path) = scan.claude_md {
            println!(
                "      {} CLAUDE.md ({} lines)",
                style("├──").dim(),
                scan.claude_md_lines
            );
        }

        if scan.claude_settings {
            println!("      {} .claude/settings.json", style("├──").dim());
        }

        if scan.mcp_json {
            let count = scan.mcp_servers.len();
            let names: Vec<&str> = scan.mcp_servers.keys().map(String::as_str).collect();
            let names_str = names.join(", ");
            println!(
                "      {} .mcp.json ({count} server{}: {names_str})",
                style("├──").dim(),
                if count == 1 { "" } else { "s" }
            );
        }

        if !scan.claude_agents.is_empty() {
            let count = scan.claude_agents.len();
            let names = scan.claude_agents.join(", ");
            println!(
                "      {} .claude/agents/ ({count} agent{}: {names})",
                style("└──").dim(),
                if count == 1 { "" } else { "s" }
            );
        }

        if !scan.claude_skills.is_empty() {
            let count = scan.claude_skills.len();
            let names = scan.claude_skills.join(", ");
            println!(
                "      {} .claude/skills/ ({count} skill{}: {names})",
                style("└──").dim(),
                if count == 1 { "" } else { "s" }
            );
        }
    }

    // Gemini
    if scan.gemini_md.is_some()
        || scan.gemini_dir
        || scan.gemini_settings
        || !scan.gemini_agents.is_empty()
        || !scan.gemini_skills.is_empty()
    {
        println!("    {} {}", style("●").green(), style("gemini-cli").bold());
        if scan.gemini_md.is_some() {
            println!(
                "      {} GEMINI.md ({} lines)",
                style("├──").dim(),
                scan.gemini_md_lines
            );
        }
        if scan.gemini_settings {
            println!("      {} .gemini/settings.json", style("├──").dim());
        }
        if !scan.gemini_agents.is_empty() {
            let count = scan.gemini_agents.len();
            let names = scan.gemini_agents.join(", ");
            println!(
                "      {} .gemini/agents/ ({count} agent{}: {names})",
                style("├──").dim(),
                if count == 1 { "" } else { "s" }
            );
        }
        if !scan.gemini_skills.is_empty() {
            let count = scan.gemini_skills.len();
            let names = scan.gemini_skills.join(", ");
            println!(
                "      {} .gemini/skills/ ({count} skill{}: {names})",
                style("├──").dim(),
                if count == 1 { "" } else { "s" }
            );
        }
        if scan.gemini_dir
            && !scan.gemini_settings
            && scan.gemini_agents.is_empty()
            && scan.gemini_skills.is_empty()
        {
            println!("      {} .gemini/", style("└──").dim());
        }
    }

    // Cursor
    if scan.cursor_dir || !scan.cursor_rules.is_empty() {
        println!("    {} {}", style("●").green(), style("cursor").bold());
        if !scan.cursor_rules.is_empty() {
            let count = scan.cursor_rules.len();
            let names = scan.cursor_rules.join(", ");
            println!(
                "      {} .cursor/rules/ ({count} rule{}: {names})",
                style("└──").dim(),
                if count == 1 { "" } else { "s" }
            );
        } else {
            println!("      {} .cursor/", style("└──").dim());
        }
    }

    // Codex
    if scan.codex_dir || !scan.codex_skills.is_empty() {
        println!("    {} {}", style("●").green(), style("codex").bold());
        if !scan.codex_skills.is_empty() {
            let count = scan.codex_skills.len();
            let names = scan.codex_skills.join(", ");
            println!(
                "      {} .codex/skills/ ({count} skill{}: {names})",
                style("└──").dim(),
                if count == 1 { "" } else { "s" }
            );
        } else {
            println!("      {} .codex/", style("└──").dim());
        }
    }

    // Windsurf
    if scan.windsurf_dir || !scan.windsurf_rules.is_empty() {
        println!("    {} {}", style("●").green(), style("windsurf").bold());
        if !scan.windsurf_rules.is_empty() {
            let count = scan.windsurf_rules.len();
            let names = scan.windsurf_rules.join(", ");
            println!(
                "      {} .windsurf/rules/ ({count} rule{}: {names})",
                style("└──").dim(),
                if count == 1 { "" } else { "s" }
            );
        } else {
            println!("      {} .windsurf/", style("└──").dim());
        }
    }

    if scan.agents_md.is_some() {
        println!();
        println!("    {} AGENTS.md", style("●").green());
    }

    println!();
}

// ===========================================================================
// Context file splitting
// ===========================================================================

/// Split CLAUDE.md content into (base, overlay) using keyword heuristics.
///
/// Sections containing Claude-specific keywords go to the overlay.
/// Everything else goes to the base. The user should review the result.
pub fn split_claude_md(content: &str) -> (String, String) {
    let sections = split_into_sections(content);

    let mut base_sections = Vec::new();
    let mut overlay_sections = Vec::new();

    for section in &sections {
        if is_claude_specific(section) {
            overlay_sections.push(section.as_str());
        } else {
            base_sections.push(section.as_str());
        }
    }

    let mut base = base_sections.join("\n\n");
    let mut overlay = overlay_sections.join("\n\n");

    // Always produce a base with the header, even if all content was Claude-specific.
    if !base.is_empty() {
        base = format!(
            "# Project Context\n\n\
             <!-- This file was auto-generated by `agx init` from CLAUDE.md. -->\n\
             <!-- Review and refine — the base/overlay split is heuristic. -->\n\n\
             {base}\n"
        );
    } else {
        base = "# Project Context\n\n\
                <!-- This file was auto-generated by `agx init` from CLAUDE.md. -->\n\
                <!-- All content was detected as Claude-specific and placed in the overlay. -->\n\
                <!-- Move any backend-agnostic content here. -->\n"
            .to_string();
    }

    if !overlay.is_empty() {
        overlay = format!(
            "# Claude Code Context\n\n\
             <!-- Claude-specific content extracted from CLAUDE.md by `agx init`. -->\n\
             <!-- Review: some sections may belong in the shared CONTEXT.md instead. -->\n\n\
             {overlay}\n"
        );
    } else {
        overlay = "# Claude Code Context\n\n\
                   <!-- Add Claude Code-specific instructions here. -->\n"
            .to_string();
    }

    (base, overlay)
}

/// Split text into sections delimited by markdown headings or blank-line-separated paragraphs.
fn split_into_sections(content: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        // A markdown heading starts a new section.
        if line.starts_with('#') && !current.trim().is_empty() {
            sections.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.trim().is_empty() {
        sections.push(current.trim().to_string());
    }

    sections
}

/// Check whether a section of text contains Claude-specific keywords.
fn is_claude_specific(text: &str) -> bool {
    let lower = text.to_lowercase();
    for kw in CLAUDE_KEYWORDS {
        if lower.contains(&kw.to_lowercase()) {
            return true;
        }
    }
    false
}

// ===========================================================================
// File writers
// ===========================================================================

/// Serialize and write the manifest to `agent-environment.yml`.
fn write_manifest(cwd: &Path, manifest: &AgentEnvironment) -> Result<()> {
    let path = cwd.join(MANIFEST_FILE);
    let content = serde_yaml::to_string(manifest).context("serializing manifest")?;
    fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Write the base CONTEXT.md template.
fn write_context_base(cwd: &Path) -> Result<()> {
    let path = cwd.join("CONTEXT.md");
    let content = "\
# Project Context

<!-- Shared project context for all backends. -->
<!-- This file is the base; backend-specific overlays add to it. -->

## Project Overview

<!-- Describe your project here. -->

## Architecture

<!-- Describe high-level architecture, key modules, data flow, etc. -->

## Coding Conventions

<!-- List coding conventions, style guides, linting rules, etc. -->

## Commands

<!-- Common build/test/run commands. -->
";
    fs::write(&path, content).context("writing CONTEXT.md")?;
    Ok(())
}

/// Write a backend-specific context overlay template.
fn write_context_overlay(cwd: &Path, backend: &str, filename: &str) -> Result<()> {
    let path = cwd.join(filename);
    let content = format!(
        "\
# {backend} Context

<!-- Instructions specific to {backend}. -->
<!-- This content is merged on top of CONTEXT.md when generating the backend config. -->
"
    );
    fs::write(&path, content).context(format!("writing {filename}"))?;
    Ok(())
}

/// Gitignore patterns keyed by backend.
fn gitignore_patterns_for_backend(backend: &str) -> Vec<&'static str> {
    match backend {
        "claude-code" => vec![".claude/", ".mcp.json", "CLAUDE.md"],
        "gemini-cli" => vec![".gemini/", "GEMINI.md"],
        "codex" => vec![".codex/"],
        "cursor" => vec![".cursor/"],
        _ => vec![],
    }
}

/// Update (or create) `.gitignore` with agx patterns, appending without duplicating.
pub fn update_gitignore(cwd: &Path, backends: &[&str], has_context: bool) -> Result<()> {
    let path = cwd.join(".gitignore");
    let existing = if path.exists() {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };

    let existing_lines: std::collections::HashSet<&str> =
        existing.lines().map(|l| l.trim()).collect();

    let mut new_lines = Vec::new();
    new_lines.push("# agx - agent environment manager");
    new_lines.push(".agx/");

    for &backend in backends {
        for pattern in gitignore_patterns_for_backend(backend) {
            new_lines.push(pattern);
        }
    }

    if has_context {
        // The generated context files (e.g. AGENTS.md) are gitignored when context is managed.
        if !new_lines.contains(&"AGENTS.md") {
            new_lines.push("AGENTS.md");
        }
    }

    // Filter out lines already present.
    let to_add: Vec<&str> = new_lines
        .into_iter()
        .filter(|line| !existing_lines.contains(line))
        .collect();

    if to_add.is_empty() {
        return Ok(());
    }

    let mut output = existing;
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    if !output.is_empty() {
        output.push('\n');
    }

    for line in to_add {
        output.push_str(line);
        output.push('\n');
    }

    fs::write(&path, &output).context("writing .gitignore")?;
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Scanner tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let scan = scan_project(dir.path());
        assert!(!scan.has_detections());
        assert!(scan.detected_backends().is_empty());
    }

    #[test]
    fn scan_detects_claude_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# Hello\nsome content\n").unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.has_detections());
        assert!(scan.claude_md.is_some());
        assert_eq!(scan.claude_md_lines, 2);
        assert!(scan.detected_backends().contains(&"claude-code"));
    }

    #[test]
    fn scan_detects_gemini_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("GEMINI.md"), "# Gemini\n").unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.has_detections());
        assert!(scan.gemini_md.is_some());
        assert!(scan.detected_backends().contains(&"gemini-cli"));
    }

    #[test]
    fn scan_detects_claude_settings() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"model": "opus"}"#,
        )
        .unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.claude_settings);
        assert!(scan.claude_settings_value.is_some());
    }

    #[test]
    fn scan_detects_mcp_json() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "mcpServers": {
                "playwright": { "url": "https://mcp.playwright.dev/sse" },
                "filesystem": {
                    "command": "npx",
                    "args": ["@anthropic-ai/mcp-server-filesystem", "./src"],
                    "env": { "ALLOW_WRITE": "true" }
                }
            }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.mcp_json);
        assert_eq!(scan.mcp_servers.len(), 2);
        assert!(scan.mcp_servers.contains_key("playwright"));
        assert!(scan.mcp_servers.contains_key("filesystem"));

        let pw = &scan.mcp_servers["playwright"];
        assert_eq!(pw.url.as_deref(), Some("https://mcp.playwright.dev/sse"));

        let fs_srv = &scan.mcp_servers["filesystem"];
        assert_eq!(fs_srv.command.as_deref(), Some("npx"));
        assert_eq!(fs_srv.args.len(), 2);
        assert_eq!(fs_srv.env.get("ALLOW_WRITE").map(|s| s.as_str()), Some("true"));
    }

    #[test]
    fn scan_detects_agents_and_skills() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/deploy-checklist")).unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/web-design")).unwrap();
        fs::write(
            dir.path().join(".claude/agents/code-reviewer.md"),
            "# Code Reviewer",
        )
        .unwrap();
        fs::write(
            dir.path().join(".claude/agents/security-auditor.md"),
            "# Security Auditor",
        )
        .unwrap();

        let scan = scan_project(dir.path());
        assert_eq!(scan.claude_agents, vec!["code-reviewer", "security-auditor"]);
        assert_eq!(scan.claude_skills, vec!["deploy-checklist", "web-design"]);
    }

    #[test]
    fn scan_detects_gemini_dir() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".gemini")).unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.gemini_dir);
        assert!(scan.detected_backends().contains(&"gemini-cli"));
    }

    // -----------------------------------------------------------------------
    // Manifest generation tests
    // -----------------------------------------------------------------------

    #[test]
    fn write_and_parse_generated_manifest() {
        let dir = TempDir::new().unwrap();

        let mut backends = BTreeMap::new();
        backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );

        let manifest = AgentEnvironment {
            name: "test-project".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends,
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        env_file: None,
        };

        write_manifest(dir.path(), &manifest).unwrap();

        // Parse it back.
        let path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.name, "test-project");
        assert!(parsed.backends.contains_key("claude-code"));
    }

    #[test]
    fn generated_manifest_with_mcp_is_valid_yaml() {
        let dir = TempDir::new().unwrap();

        let mut backends = BTreeMap::new();
        backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );

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

        let manifest = AgentEnvironment {
            name: "test-with-mcp".to_string(),
            version: None,
            description: None,
            backends,
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers,
            settings: BTreeMap::new(),
            paths: None,
            context: Some(ContextConfig {
                base: "./CONTEXT.md".to_string(),
                overlays: {
                    let mut m = BTreeMap::new();
                    m.insert(
                        "claude-code".to_string(),
                        "./CONTEXT.claude-code.md".to_string(),
                    );
                    m
                },
                children: vec![],
            }),
            hooks: None,
        env_file: None,
        };

        write_manifest(dir.path(), &manifest).unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.mcp_servers.len(), 1);
        assert!(parsed.context.is_some());
    }

    // -----------------------------------------------------------------------
    // .gitignore tests
    // -----------------------------------------------------------------------

    #[test]
    fn gitignore_created_when_absent() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["claude-code"], true).unwrap();

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".agx/"));
        assert!(content.contains(".claude/"));
        assert!(content.contains(".mcp.json"));
        assert!(content.contains("CLAUDE.md"));
    }

    #[test]
    fn gitignore_appends_without_duplicating() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules/\n.agx/\n").unwrap();

        update_gitignore(dir.path(), &["claude-code"], false).unwrap();

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        // .agx/ should appear only once (from existing)
        let agx_count = content.matches(".agx/").count();
        assert_eq!(agx_count, 1, "should not duplicate .agx/");
        // But new patterns should be added
        assert!(content.contains(".claude/"));
        assert!(content.contains("node_modules/"));
    }

    #[test]
    fn gitignore_backend_specific_patterns() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["gemini-cli", "codex"], false).unwrap();

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".gemini/"));
        assert!(content.contains("GEMINI.md"));
        assert!(content.contains(".codex/"));
        // Should NOT contain Claude patterns
        assert!(!content.contains(".claude/"));
    }

    // -----------------------------------------------------------------------
    // Context splitting tests
    // -----------------------------------------------------------------------

    #[test]
    fn split_claude_md_basic() {
        let content = "\
# Project Overview

This is a Rust web server project.

## Architecture

Uses actix-web for HTTP.

## Claude Code Tips

Use /compact to save context. Configure MCP servers via .claude/ directory.

## Build Commands

cargo build
cargo test
";

        let (base, overlay) = split_claude_md(content);

        // The Claude-specific section should go to overlay.
        assert!(overlay.contains("/compact"));
        assert!(overlay.contains("MCP server"));

        // Generic sections should go to base.
        assert!(base.contains("Rust web server"));
        assert!(base.contains("actix-web"));
        assert!(base.contains("cargo build"));
    }

    #[test]
    fn split_claude_md_all_generic() {
        let content = "\
# My Project

A simple project with no Claude-specific content.

## Commands

make build
make test
";

        let (base, overlay) = split_claude_md(content);
        assert!(base.contains("simple project"));
        assert!(base.contains("make build"));
        // Overlay should just be the placeholder.
        assert!(overlay.contains("Claude Code Context"));
    }

    #[test]
    fn split_claude_md_all_specific() {
        let content = "\
# Claude Code Setup

Use /compact regularly. Configure .claude/ settings for your project.
Use the Bash() and Read() tools effectively.
";

        let (base, overlay) = split_claude_md(content);
        assert!(overlay.contains("/compact"));
        assert!(overlay.contains(".claude/"));
        // Base should have the header but no content sections.
        assert!(base.contains("Project Context"));
    }

    // -----------------------------------------------------------------------
    // --force and --scan-only behavior tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_only_does_not_create_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# Hello").unwrap();

        let scan = scan_project(dir.path());
        // Just verify the scan works — the actual --scan-only flag only prints, no files.
        assert!(scan.has_detections());
        assert!(!dir.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn force_skips_detection() {
        // With --force, even if config exists, we'd run new-project flow.
        // Test that scan results are irrelevant when force is set.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# existing").unwrap();

        let scan = scan_project(dir.path());
        assert!(scan.has_detections());
        // In the actual run(), --force would skip import flow. We verify the
        // scan data is correct but unused — the logic is in run().
    }

    // -----------------------------------------------------------------------
    // MCP JSON parsing edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn parse_mcp_json_flat_format() {
        // Some .mcp.json files may not have the mcpServers wrapper.
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "myserver": { "command": "node", "args": ["server.js"] }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("myserver"));
    }

    #[test]
    fn parse_mcp_json_invalid_gracefully() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".mcp.json"), "not valid json!!!").unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_mcp_json_missing_file() {
        let dir = TempDir::new().unwrap();
        let servers = parse_mcp_json(&dir.path().join("nonexistent.json"));
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_mcp_json_with_mcp_servers_wrapper() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "mcpServers": {
                "server-a": { "url": "https://a.example.com" },
                "server-b": { "command": "python", "args": ["-m", "server"] }
            }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["server-a"].url.as_deref(), Some("https://a.example.com"));
        assert_eq!(servers["server-b"].command.as_deref(), Some("python"));
        assert_eq!(servers["server-b"].args, vec!["-m", "server"]);
    }

    #[test]
    fn parse_mcp_json_direct_server_map() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "direct-server": {
                "command": "node",
                "args": ["index.js"],
                "env": { "PORT": "3000" }
            }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert_eq!(servers.len(), 1);
        let srv = &servers["direct-server"];
        assert_eq!(srv.command.as_deref(), Some("node"));
        assert_eq!(srv.args, vec!["index.js"]);
        assert_eq!(srv.env.get("PORT").map(|s| s.as_str()), Some("3000"));
    }

    #[test]
    fn parse_mcp_json_empty_object() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".mcp.json"), "{}").unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_mcp_json_server_with_no_command_or_url() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{ "mcpServers": { "empty-srv": {} } }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert_eq!(servers.len(), 1);
        let srv = &servers["empty-srv"];
        assert!(srv.url.is_none());
        assert!(srv.command.is_none());
        assert!(srv.args.is_empty());
        assert!(srv.env.is_empty());
    }

    // -----------------------------------------------------------------------
    // split_claude_md edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn split_claude_md_empty_content() {
        let (base, overlay) = split_claude_md("");
        // Base should have the "all content was Claude-specific" header since there are no base sections.
        assert!(base.contains("Project Context"));
        // Overlay should be the placeholder.
        assert!(overlay.contains("Claude Code Context"));
        assert!(overlay.contains("Add Claude Code-specific instructions here"));
    }

    #[test]
    fn split_claude_md_only_headers() {
        let content = "# Header One\n# Header Two\n# Header Three\n";
        let (base, overlay) = split_claude_md(content);
        // None of these headers contain Claude keywords, so they go to base.
        assert!(base.contains("Header One"));
        assert!(base.contains("Header Two"));
        assert!(base.contains("Header Three"));
        // Overlay is just the placeholder.
        assert!(overlay.contains("Add Claude Code-specific instructions here"));
    }

    #[test]
    fn split_claude_md_nested_headers() {
        let content = "\
# Top Level

Some general content.

## Sub Section

More general info.

### Deep Section with Claude Code

Use /compact to save tokens. Run Bash() for shell access.

## Another Sub

Generic sub content.
";
        let (base, overlay) = split_claude_md(content);
        // The deep section mentions /compact and Bash( => overlay
        assert!(overlay.contains("/compact"));
        assert!(overlay.contains("Bash()"));
        // The other sections should be in base
        assert!(base.contains("general content"));
        assert!(base.contains("Generic sub content"));
    }

    #[test]
    fn split_claude_md_no_separable_sections() {
        // Content with no headings at all — everything is one section.
        let content = "Just some plain text with no markdown headings at all.\nSecond line.\n";
        let (base, overlay) = split_claude_md(content);
        // No claude keywords => all goes to base.
        assert!(base.contains("plain text"));
        assert!(overlay.contains("Add Claude Code-specific instructions here"));
    }

    #[test]
    fn split_claude_md_single_claude_specific_section_no_headings() {
        let content = "Always use Read() and Edit() tools. Configure .claude/ directory.\n";
        let (base, overlay) = split_claude_md(content);
        // Contains Claude keywords => goes to overlay. Base is empty placeholder.
        assert!(overlay.contains("Read()"));
        assert!(overlay.contains(".claude/"));
        assert!(base.contains("All content was detected as Claude-specific"));
    }

    // -----------------------------------------------------------------------
    // update_gitignore additional tests
    // -----------------------------------------------------------------------

    #[test]
    fn gitignore_all_known_backends() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["claude-code", "gemini-cli", "codex", "cursor"], true).unwrap();

        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".agx/"));
        assert!(content.contains(".claude/"));
        assert!(content.contains(".mcp.json"));
        assert!(content.contains("CLAUDE.md"));
        assert!(content.contains(".gemini/"));
        assert!(content.contains("GEMINI.md"));
        assert!(content.contains(".codex/"));
        assert!(content.contains(".cursor/"));
        assert!(content.contains("AGENTS.md"));
    }

    #[test]
    fn gitignore_with_context_adds_agents_md() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["claude-code"], true).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("AGENTS.md"));
    }

    #[test]
    fn gitignore_without_context_no_agents_md() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["claude-code"], false).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(!content.contains("AGENTS.md"));
    }

    #[test]
    fn gitignore_unknown_backend_no_extra_patterns() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &["unknown-backend"], false).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".agx/"));
        // No backend-specific patterns for unknown backend.
        assert!(!content.contains(".claude/"));
        assert!(!content.contains(".gemini/"));
    }

    #[test]
    fn gitignore_noop_when_all_present() {
        let dir = TempDir::new().unwrap();
        // Write .gitignore with all patterns already present.
        update_gitignore(dir.path(), &["claude-code"], false).unwrap();
        let content1 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        // Call again — should not change anything.
        update_gitignore(dir.path(), &["claude-code"], false).unwrap();
        let content2 = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content1, content2);
    }

    // -----------------------------------------------------------------------
    // write_manifest tests
    // -----------------------------------------------------------------------

    #[test]
    fn write_manifest_with_all_fields() {
        let dir = TempDir::new().unwrap();
        let mut backends = BTreeMap::new();
        backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
        backends.insert(
            "gemini-cli".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Permissive),
                model: None,
            },
        );

        let mut mcp_servers = BTreeMap::new();
        mcp_servers.insert(
            "test-mcp".to_string(),
            McpServerConfig {
                url: None,
                command: Some("node".to_string()),
                args: vec!["server.js".to_string()],
                env: BTreeMap::new(),
            },
        );

        let mut skills = BTreeMap::new();
        skills.insert(
            "my-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/my-skill".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        let manifest = AgentEnvironment {
            name: "full-project".to_string(),
            version: Some("1.0.0".to_string()),
            description: Some("A full project".to_string()),
            backends,
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills,
            agents: BTreeMap::new(),
            mcp_servers,
            settings: BTreeMap::new(),
            paths: None,
            context: Some(ContextConfig {
                base: "./CONTEXT.md".to_string(),
                overlays: {
                    let mut m = BTreeMap::new();
                    m.insert("claude-code".to_string(), "./CONTEXT.claude-code.md".to_string());
                    m
                },
                children: vec![],
            }),
            hooks: None,
        env_file: None,
        };

        write_manifest(dir.path(), &manifest).unwrap();

        let path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.name, "full-project");
        assert_eq!(parsed.backends.len(), 2);
        assert_eq!(parsed.mcp_servers.len(), 1);
        assert_eq!(parsed.skills.len(), 1);
        assert!(parsed.context.is_some());
    }

    // -----------------------------------------------------------------------
    // write_context_base and write_context_overlay tests
    // -----------------------------------------------------------------------

    #[test]
    fn write_context_base_creates_file() {
        let dir = TempDir::new().unwrap();
        write_context_base(dir.path()).unwrap();
        let path = dir.path().join("CONTEXT.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Project Context"));
        assert!(content.contains("Project Overview"));
    }

    #[test]
    fn write_context_overlay_creates_file() {
        let dir = TempDir::new().unwrap();
        write_context_overlay(dir.path(), "claude-code", "CONTEXT.claude-code.md").unwrap();
        let path = dir.path().join("CONTEXT.claude-code.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("claude-code Context"));
    }

    // -----------------------------------------------------------------------
    // gitignore_patterns_for_backend tests
    // -----------------------------------------------------------------------

    #[test]
    fn gitignore_patterns_claude_code() {
        let patterns = gitignore_patterns_for_backend("claude-code");
        assert!(patterns.contains(&".claude/"));
        assert!(patterns.contains(&".mcp.json"));
        assert!(patterns.contains(&"CLAUDE.md"));
    }

    #[test]
    fn gitignore_patterns_gemini_cli() {
        let patterns = gitignore_patterns_for_backend("gemini-cli");
        assert!(patterns.contains(&".gemini/"));
        assert!(patterns.contains(&"GEMINI.md"));
    }

    #[test]
    fn gitignore_patterns_codex() {
        let patterns = gitignore_patterns_for_backend("codex");
        assert!(patterns.contains(&".codex/"));
    }

    #[test]
    fn gitignore_patterns_cursor() {
        let patterns = gitignore_patterns_for_backend("cursor");
        assert!(patterns.contains(&".cursor/"));
    }

    #[test]
    fn gitignore_patterns_unknown() {
        let patterns = gitignore_patterns_for_backend("unknown");
        assert!(patterns.is_empty());
    }

    // -----------------------------------------------------------------------
    // ScanResult tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_result_detected_backends_both() {
        let scan = ScanResult {
            claude_md: Some(PathBuf::from("CLAUDE.md")),
            claude_md_lines: 10,
            gemini_md: Some(PathBuf::from("GEMINI.md")),
            gemini_md_lines: 5,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        let backends = scan.detected_backends();
        assert!(backends.contains(&"claude-code"));
        assert!(backends.contains(&"gemini-cli"));
    }

    #[test]
    fn scan_result_has_detections_agents_md() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: Some(PathBuf::from("AGENTS.md")),
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.has_detections());
    }

    #[test]
    fn scan_result_detected_backends_from_settings() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: true,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.detected_backends().contains(&"claude-code"));
    }

    #[test]
    fn scan_result_detected_backends_from_mcp() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: true,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.detected_backends().contains(&"claude-code"));
    }

    #[test]
    fn scan_result_detected_backends_from_agents() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: vec!["my-agent".to_string()],
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.detected_backends().contains(&"claude-code"));
    }

    #[test]
    fn scan_result_detected_backends_from_skills() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: vec!["my-skill".to_string()],
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.detected_backends().contains(&"claude-code"));
    }

    #[test]
    fn scan_result_detected_backends_from_gemini_dir() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: true,
            ..Default::default()
        };
        assert!(scan.detected_backends().contains(&"gemini-cli"));
    }

    // -----------------------------------------------------------------------
    // is_claude_specific tests
    // -----------------------------------------------------------------------

    #[test]
    fn is_claude_specific_detects_keywords() {
        assert!(is_claude_specific("Use /compact to save context"));
        assert!(is_claude_specific("Configure MCP server for tools"));
        assert!(is_claude_specific("Use the Bash() tool"));
        assert!(is_claude_specific("Edit CLAUDE.md for instructions"));
        assert!(is_claude_specific("Use extended thinking for complex tasks"));
        assert!(is_claude_specific("Run TodoWrite to track tasks"));
        assert!(is_claude_specific("Configure .claude/ directory"));
        assert!(is_claude_specific("Use a slash command to interact"));
    }

    #[test]
    fn is_claude_specific_returns_false_for_generic() {
        assert!(!is_claude_specific("Run cargo build to compile"));
        assert!(!is_claude_specific("Use make test to run tests"));
        assert!(!is_claude_specific("The architecture uses microservices"));
    }

    // -----------------------------------------------------------------------
    // split_into_sections tests
    // -----------------------------------------------------------------------

    #[test]
    fn split_into_sections_basic() {
        let content = "# Section 1\nContent 1\n# Section 2\nContent 2\n";
        let sections = split_into_sections(content);
        assert_eq!(sections.len(), 2);
        assert!(sections[0].contains("Section 1"));
        assert!(sections[1].contains("Section 2"));
    }

    #[test]
    fn split_into_sections_no_headings() {
        let content = "Just plain text\nwith multiple lines\n";
        let sections = split_into_sections(content);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].contains("plain text"));
    }

    #[test]
    fn split_into_sections_empty() {
        let sections = split_into_sections("");
        assert!(sections.is_empty());
    }

    // -----------------------------------------------------------------------
    // print_scan_results coverage (just call it, no assertion on stdout)
    // -----------------------------------------------------------------------

    #[test]
    fn print_scan_results_no_detections() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Just exercise the function for coverage.
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_with_all_detections() {
        let mut mcp_servers = BTreeMap::new();
        mcp_servers.insert(
            "test-server".to_string(),
            McpServerConfig {
                url: Some("https://example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let scan = ScanResult {
            claude_md: Some(PathBuf::from("CLAUDE.md")),
            claude_md_lines: 42,
            gemini_md: Some(PathBuf::from("GEMINI.md")),
            gemini_md_lines: 10,
            agents_md: Some(PathBuf::from("AGENTS.md")),
            claude_settings: true,
            claude_settings_value: None,
            mcp_json: true,
            mcp_servers,
            claude_agents: vec!["reviewer".to_string()],
            claude_skills: vec!["deploy".to_string()],
            gemini_dir: true,
            ..Default::default()
        };
        // Exercise the function for coverage.
        print_scan_results(&scan);
    }

    // -----------------------------------------------------------------------
    // print_scan_results additional coverage
    // -----------------------------------------------------------------------

    #[test]
    fn print_scan_results_claude_only_no_agents_no_skills() {
        let scan = ScanResult {
            claude_md: Some(PathBuf::from("CLAUDE.md")),
            claude_md_lines: 5,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises the claude-code branch without settings/mcp/agents/skills.
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_gemini_only_no_gemini_md() {
        // gemini_dir=true but gemini_md=None — exercises the gemini branch without GEMINI.md
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: true,
            ..Default::default()
        };
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_multiple_mcp_servers() {
        let mut mcp_servers = BTreeMap::new();
        mcp_servers.insert(
            "server-a".to_string(),
            McpServerConfig {
                url: Some("https://a.example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        mcp_servers.insert(
            "server-b".to_string(),
            McpServerConfig {
                url: Some("https://b.example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: true,
            mcp_servers,
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises the mcp_json branch with count > 1 (plural "servers")
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_single_mcp_server() {
        let mut mcp_servers = BTreeMap::new();
        mcp_servers.insert(
            "only-server".to_string(),
            McpServerConfig {
                url: Some("https://only.example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: true,
            mcp_servers,
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises the mcp_json branch with count == 1 (singular "server")
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_single_agent() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: vec!["solo-agent".to_string()],
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises the agents branch with count == 1 (singular "agent")
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_single_skill() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: vec!["solo-skill".to_string()],
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises the skills branch with count == 1 (singular "skill")
        print_scan_results(&scan);
    }

    // -----------------------------------------------------------------------
    // scan_project additional coverage
    // -----------------------------------------------------------------------

    #[test]
    fn scan_detects_agents_md() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "# Agents\n").unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.has_detections());
        assert!(scan.agents_md.is_some());
    }

    #[test]
    fn scan_agents_dir_ignores_non_md_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::write(dir.path().join(".claude/agents/good.md"), "# Agent").unwrap();
        fs::write(dir.path().join(".claude/agents/ignored.txt"), "not an agent").unwrap();
        fs::write(dir.path().join(".claude/agents/also-ignored.json"), "{}").unwrap();
        let scan = scan_project(dir.path());
        assert_eq!(scan.claude_agents, vec!["good"]);
    }

    #[test]
    fn scan_skills_dir_ignores_files() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/real-skill")).unwrap();
        fs::write(dir.path().join(".claude/skills/not-a-skill.txt"), "file").unwrap();
        let scan = scan_project(dir.path());
        assert_eq!(scan.claude_skills, vec!["real-skill"]);
    }

    #[test]
    fn scan_claude_settings_invalid_json() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(dir.path().join(".claude/settings.json"), "not valid json").unwrap();
        let scan = scan_project(dir.path());
        assert!(scan.claude_settings);
        assert!(scan.claude_settings_value.is_none());
    }

    #[test]
    fn scan_full_project_with_everything() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# Claude\nLine 2\nLine 3\n").unwrap();
        fs::write(dir.path().join("GEMINI.md"), "# Gemini\nLine 2\n").unwrap();
        fs::write(dir.path().join("AGENTS.md"), "# Agents\n").unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/my-skill")).unwrap();
        fs::write(dir.path().join(".claude/agents/helper.md"), "# Helper").unwrap();
        fs::write(dir.path().join(".claude/settings.json"), r#"{"key": "val"}"#).unwrap();
        let mcp = r#"{"mcpServers": {"srv": {"url": "https://example.com"}}}"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        fs::create_dir_all(dir.path().join(".gemini")).unwrap();

        let scan = scan_project(dir.path());
        assert!(scan.has_detections());
        assert!(scan.claude_md.is_some());
        assert_eq!(scan.claude_md_lines, 3);
        assert!(scan.gemini_md.is_some());
        assert_eq!(scan.gemini_md_lines, 2);
        assert!(scan.agents_md.is_some());
        assert!(scan.claude_settings);
        assert!(scan.claude_settings_value.is_some());
        assert!(scan.mcp_json);
        assert_eq!(scan.mcp_servers.len(), 1);
        assert_eq!(scan.claude_agents, vec!["helper"]);
        assert_eq!(scan.claude_skills, vec!["my-skill"]);
        assert!(scan.gemini_dir);

        let backends = scan.detected_backends();
        assert!(backends.contains(&"claude-code"));
        assert!(backends.contains(&"gemini-cli"));
    }

    // -----------------------------------------------------------------------
    // write_manifest edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn write_manifest_with_registries() {
        let dir = TempDir::new().unwrap();

        let mut registries = BTreeMap::new();
        registries.insert(
            "my-registry".to_string(),
            RegistryConfig {
                url: "https://registry.example.com".to_string(),
                auth: None,
                pin: None,
            },
        );

        let manifest = AgentEnvironment {
            name: "registry-project".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends: BTreeMap::new(),
            registries,
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        env_file: None,
        };

        write_manifest(dir.path(), &manifest).unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.registries.len(), 1);
        assert_eq!(parsed.registries["my-registry"].url, "https://registry.example.com");
    }

    #[test]
    fn write_manifest_with_agents() {
        let dir = TempDir::new().unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "code-reviewer".to_string(),
            AgentDependency {
                git: None,
                path: "./.claude/agents/code-reviewer.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );

        let manifest = AgentEnvironment {
            name: "agents-project".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents,
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        env_file: None,
        };

        write_manifest(dir.path(), &manifest).unwrap();
        let path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.agents.len(), 1);
        assert!(parsed.agents.contains_key("code-reviewer"));
    }

    // -----------------------------------------------------------------------
    // write_context_overlay additional coverage
    // -----------------------------------------------------------------------

    #[test]
    fn write_context_overlay_gemini() {
        let dir = TempDir::new().unwrap();
        write_context_overlay(dir.path(), "gemini-cli", "CONTEXT.gemini-cli.md").unwrap();
        let path = dir.path().join("CONTEXT.gemini-cli.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("gemini-cli Context"));
        assert!(content.contains("specific to gemini-cli"));
    }

    // -----------------------------------------------------------------------
    // update_gitignore edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn gitignore_appends_to_file_without_trailing_newline() {
        let dir = TempDir::new().unwrap();
        // Write a .gitignore that does NOT end with a newline
        fs::write(dir.path().join(".gitignore"), "node_modules/").unwrap();
        update_gitignore(dir.path(), &["claude-code"], false).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(".agx/"));
        assert!(content.contains(".claude/"));
    }

    #[test]
    fn gitignore_windsurf_backend_has_no_patterns() {
        // windsurf is not in gitignore_patterns_for_backend currently
        let patterns = gitignore_patterns_for_backend("windsurf");
        assert!(patterns.is_empty());
    }

    #[test]
    fn gitignore_empty_backends_list() {
        let dir = TempDir::new().unwrap();
        update_gitignore(dir.path(), &[], false).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".agx/"));
        // No backend-specific patterns
        assert!(!content.contains(".claude/"));
        assert!(!content.contains(".gemini/"));
    }

    // -----------------------------------------------------------------------
    // split_claude_md additional coverage
    // -----------------------------------------------------------------------

    #[test]
    fn split_claude_md_case_insensitive_keyword_match() {
        let content = "\
# Setup

Configure your CLAUDE CODE environment properly.
";
        let (base, overlay) = split_claude_md(content);
        // "claude code" (case insensitive) should be detected
        assert!(overlay.contains("CLAUDE CODE"));
        assert!(base.contains("All content was detected as Claude-specific"));
    }

    #[test]
    fn split_claude_md_mixed_sections_complex() {
        let content = "\
# Introduction

This project uses Rust.

## Build Steps

cargo build
cargo test

## Memory Management

Use claude memory feature and /memory command.

## Testing

Use cargo test with --release.

## Tool Usage

Always prefer the Read() and Grep() tools.

## Deployment

Deploy via Docker.
";
        let (base, overlay) = split_claude_md(content);
        // Generic sections
        assert!(base.contains("This project uses Rust"));
        assert!(base.contains("cargo build"));
        assert!(base.contains("Deploy via Docker"));
        // Claude-specific sections
        assert!(overlay.contains("claude memory"));
        assert!(overlay.contains("/memory"));
        assert!(overlay.contains("Read()"));
        assert!(overlay.contains("Grep()"));
    }

    #[test]
    fn split_into_sections_heading_at_start() {
        // When the first line is a heading, should still produce one section
        let content = "# Only Section\nSome text\n";
        let sections = split_into_sections(content);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].contains("Only Section"));
        assert!(sections[0].contains("Some text"));
    }

    #[test]
    fn split_into_sections_multiple_heading_levels() {
        let content = "# H1\nContent A\n## H2\nContent B\n### H3\nContent C\n";
        let sections = split_into_sections(content);
        assert_eq!(sections.len(), 3);
    }

    // -----------------------------------------------------------------------
    // is_claude_specific additional keywords
    // -----------------------------------------------------------------------

    #[test]
    fn is_claude_specific_all_keywords() {
        // Verify each keyword individually
        for kw in CLAUDE_KEYWORDS {
            assert!(
                is_claude_specific(&format!("Some text containing {} here", kw)),
                "Keyword '{}' was not detected",
                kw
            );
        }
    }

    #[test]
    fn is_claude_specific_case_insensitive() {
        assert!(is_claude_specific("use EXTENDED THINKING for tasks"));
        assert!(is_claude_specific("configure MCP SERVER properly"));
        assert!(is_claude_specific("TODOWRITE for tracking"));
    }

    // -----------------------------------------------------------------------
    // ScanResult has_detections / detected_backends edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn scan_result_no_detections_empty() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(!scan.has_detections());
        assert!(scan.detected_backends().is_empty());
    }

    #[test]
    fn scan_result_detected_backends_order() {
        // When both claude and gemini are detected, claude-code comes first
        let scan = ScanResult {
            claude_md: Some(PathBuf::from("CLAUDE.md")),
            claude_md_lines: 1,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: true,
            ..Default::default()
        };
        let backends = scan.detected_backends();
        assert_eq!(backends.len(), 2);
        assert_eq!(backends[0], "claude-code");
        assert_eq!(backends[1], "gemini-cli");
    }

    // -----------------------------------------------------------------------
    // parse_mcp_json additional edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn parse_mcp_json_server_with_empty_args_and_env() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "mcpServers": {
                "minimal": {
                    "command": "node",
                    "args": [],
                    "env": {}
                }
            }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        assert_eq!(servers.len(), 1);
        let srv = &servers["minimal"];
        assert_eq!(srv.command.as_deref(), Some("node"));
        assert!(srv.args.is_empty());
        assert!(srv.env.is_empty());
    }

    #[test]
    fn parse_mcp_json_multiple_env_vars() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{
            "mcpServers": {
                "envtest": {
                    "command": "python",
                    "args": ["serve.py"],
                    "env": {
                        "DB_HOST": "localhost",
                        "DB_PORT": "5432",
                        "DB_NAME": "mydb"
                    }
                }
            }
        }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        let srv = &servers["envtest"];
        assert_eq!(srv.env.len(), 3);
        assert_eq!(srv.env["DB_HOST"], "localhost");
        assert_eq!(srv.env["DB_PORT"], "5432");
        assert_eq!(srv.env["DB_NAME"], "mydb");
    }

    // -----------------------------------------------------------------------
    // run() tests — non-interactive mode (--yes)
    // -----------------------------------------------------------------------

    #[test]
    fn run_non_interactive_creates_manifest_and_context() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let tmp_cache = TempDir::new().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp_cache.path()) };

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: false,
            non_interactive: true,
            name: Some("test-ni".to_string()),
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_ok(), "run non-interactive failed: {:?}", result.err());

        // Manifest should be created
        let manifest_path = dir.path().join(MANIFEST_FILE);
        assert!(manifest_path.exists(), "manifest not created");
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(parsed.name, "test-ni");
        assert!(parsed.backends.contains_key("claude-code"));
        assert_eq!(parsed.version.as_deref(), Some("0.1.0"));

        // Context files should be created
        assert!(dir.path().join("CONTEXT.md").exists());
        assert!(dir.path().join("CONTEXT.claude-code.md").exists());

        // .gitignore should be created
        assert!(dir.path().join(".gitignore").exists());
    }

    #[test]
    fn run_non_interactive_custom_backends() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let tmp_cache = TempDir::new().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp_cache.path()) };

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: false,
            non_interactive: true,
            name: Some("multi-backend".to_string()),
            backend: vec!["claude-code".to_string(), "gemini-cli".to_string()],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_ok());

        let manifest_path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert!(parsed.backends.contains_key("claude-code"));
        assert!(parsed.backends.contains_key("gemini-cli"));
        // First backend should be default
        assert_eq!(parsed.backends["claude-code"].default, Some(true));
        assert_eq!(parsed.backends["gemini-cli"].default, None);

        // Both overlay files should exist
        assert!(dir.path().join("CONTEXT.claude-code.md").exists());
        assert!(dir.path().join("CONTEXT.gemini-cli.md").exists());
    }

    #[test]
    fn run_non_interactive_defaults_name_from_dir() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let tmp_cache = TempDir::new().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp_cache.path()) };

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: false,
            non_interactive: true,
            name: None,  // Should default to directory name
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_ok());

        let manifest_path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        // Name should be derived from the dir name
        assert!(!parsed.name.is_empty());
    }

    #[test]
    fn run_errors_when_manifest_exists_no_force() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(MANIFEST_FILE), "name: existing\n").unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: false,
            non_interactive: true,
            name: None,
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("already exists"));
    }

    #[test]
    fn run_force_overwrites_existing_manifest() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let tmp_cache = TempDir::new().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp_cache.path()) };

        fs::write(dir.path().join(MANIFEST_FILE), "name: old\nbackends:\n  claude-code: {}\n").unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: true,
            scan_only: false,
            non_interactive: true,
            name: Some("new-name".to_string()),
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_ok());

        let manifest_path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(parsed.name, "new-name");
    }

    #[test]
    fn run_scan_only_does_not_create_manifest() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        // Create some detectable files
        fs::write(dir.path().join("CLAUDE.md"), "# Test").unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: true,
            non_interactive: false,
            name: None,
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
        assert!(!dir.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn run_scan_only_empty_dir() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: true,
            non_interactive: false,
            name: None,
            backend: vec![],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // run_non_interactive_flow additional scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn non_interactive_manifest_has_context_config() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let tmp_cache = TempDir::new().unwrap();
        unsafe { std::env::set_var("AGX_CACHE_DIR", tmp_cache.path()) };

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = InitArgs {
            force: false,
            scan_only: false,
            non_interactive: true,
            name: Some("ctx-test".to_string()),
            backend: vec!["claude-code".to_string()],
        };

        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();
        unsafe { std::env::remove_var("AGX_CACHE_DIR") };

        assert!(result.is_ok());

        let manifest_path = dir.path().join(MANIFEST_FILE);
        let parsed: AgentEnvironment =
            serde_yaml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let ctx = parsed.context.unwrap();
        assert_eq!(ctx.base, "./CONTEXT.md");
        assert!(ctx.overlays.contains_key("claude-code"));
        assert_eq!(ctx.overlays["claude-code"], "./CONTEXT.claude-code.md");
    }

    // -----------------------------------------------------------------------
    // print_scan_results additional coverage: multiple agents/skills
    // -----------------------------------------------------------------------

    #[test]
    fn print_scan_results_multiple_agents() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises plural "agents" path
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_multiple_skills() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: vec!["skill-a".to_string(), "skill-b".to_string()],
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises plural "skills" path
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_gemini_md_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: Some(PathBuf::from("GEMINI.md")),
            gemini_md_lines: 15,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises gemini_md path without gemini_dir
        print_scan_results(&scan);
    }

    #[test]
    fn print_scan_results_agents_md_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: Some(PathBuf::from("AGENTS.md")),
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        // Exercises agents_md path
        print_scan_results(&scan);
    }

    // -----------------------------------------------------------------------
    // parse_mcp_json: mcpServers key with non-object values
    // -----------------------------------------------------------------------

    #[test]
    fn parse_mcp_json_non_object_mcp_servers() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"{ "mcpServers": "not an object" }"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        // Falls through to json.as_object() fallback, but that's also not server entries
        // mcpServers is not an object, and the top-level object has mcpServers as a string
        // so fallback to top-level as_object gives {"mcpServers": ...}
        // which would try to parse "mcpServers" as a server name
        // This exercises the fallback path
        assert!(servers.len() <= 1);
    }

    #[test]
    fn parse_mcp_json_array_value() {
        let dir = TempDir::new().unwrap();
        let mcp = r#"[1, 2, 3]"#;
        fs::write(dir.path().join(".mcp.json"), mcp).unwrap();
        let servers = parse_mcp_json(&dir.path().join(".mcp.json"));
        // Not an object at all
        assert!(servers.is_empty());
    }

    // -----------------------------------------------------------------------
    // scan_project: edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn scan_project_multiple_agents_sorted() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        fs::write(dir.path().join(".claude/agents/z-agent.md"), "").unwrap();
        fs::write(dir.path().join(".claude/agents/a-agent.md"), "").unwrap();
        fs::write(dir.path().join(".claude/agents/m-agent.md"), "").unwrap();
        let scan = scan_project(dir.path());
        assert_eq!(scan.claude_agents, vec!["a-agent", "m-agent", "z-agent"]);
    }

    #[test]
    fn scan_project_multiple_skills_sorted() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/z-skill")).unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/a-skill")).unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/m-skill")).unwrap();
        let scan = scan_project(dir.path());
        assert_eq!(scan.claude_skills, vec!["a-skill", "m-skill", "z-skill"]);
    }

    // -----------------------------------------------------------------------
    // split_claude_md: more keyword patterns
    // -----------------------------------------------------------------------

    #[test]
    fn split_claude_md_mention_keyword() {
        let content = "\
# Collaboration

Use @mention to tag other agents.
";
        let (_base, overlay) = split_claude_md(content);
        assert!(overlay.contains("@mention"));
    }

    #[test]
    fn split_claude_md_todoread_keyword() {
        let content = "\
# Tasks

Use TodoRead to check progress.
";
        let (_base, overlay) = split_claude_md(content);
        assert!(overlay.contains("TodoRead"));
    }

    #[test]
    fn split_claude_md_login_logout_keywords() {
        let content = "\
# Auth

Use /login and /logout to manage sessions.

# Build

npm run build
";
        let (base, overlay) = split_claude_md(content);
        assert!(overlay.contains("/login"));
        assert!(base.contains("npm run build"));
    }

    // -----------------------------------------------------------------------
    // has_detections: more combinations
    // -----------------------------------------------------------------------

    #[test]
    fn scan_result_has_detections_gemini_dir_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: true,
            ..Default::default()
        };
        assert!(scan.has_detections());
    }

    #[test]
    fn scan_result_has_detections_claude_settings_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: true,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.has_detections());
    }

    #[test]
    fn scan_result_has_detections_mcp_json_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: true,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: Vec::new(),
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.has_detections());
    }

    #[test]
    fn scan_result_has_detections_claude_skills_only() {
        let scan = ScanResult {
            claude_md: None,
            claude_md_lines: 0,
            gemini_md: None,
            gemini_md_lines: 0,
            agents_md: None,
            claude_settings: false,
            claude_settings_value: None,
            mcp_json: false,
            mcp_servers: BTreeMap::new(),
            claude_agents: Vec::new(),
            claude_skills: vec!["s".to_string()],
            gemini_dir: false,
            ..Default::default()
        };
        assert!(scan.has_detections());
    }

    // -----------------------------------------------------------------------
    // gitignore edge case: agents_md line already present
    // -----------------------------------------------------------------------

    #[test]
    fn gitignore_agents_md_not_duplicated_when_in_backend_patterns() {
        let dir = TempDir::new().unwrap();
        // Write .gitignore that already has AGENTS.md
        fs::write(dir.path().join(".gitignore"), "AGENTS.md\n").unwrap();
        update_gitignore(dir.path(), &["claude-code"], true).unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        let count = content.matches("AGENTS.md").count();
        assert_eq!(count, 1, "AGENTS.md should not be duplicated");
    }
}
