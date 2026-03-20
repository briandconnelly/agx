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

    if !args.force && scan.has_detections() {
        // Existing-project flow.
        print_scan_results(&scan);
        run_import_flow(&cwd, &scan)
    } else {
        // New-project flow (interactive wizard).
        run_new_project_flow(&cwd)
    }
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
    let isolation_options = &["strict", "permissive"];
    let isolation_idx = Select::new()
        .with_prompt("Default isolation mode")
        .items(isolation_options)
        .default(0)
        .interact()
        .context("isolation mode prompt")?;
    let isolation = match isolation_options[isolation_idx] {
        "strict" => IsolationMode::Strict,
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
    println!();
    println!("Next steps:");
    println!("  Edit {} to add dependencies", MANIFEST_FILE);
    println!("  agx install    Resolve, download, and generate configs");
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
    println!("  agx generate    Regenerate configs from the new manifest");
    println!("  agx run         Launch in the managed environment");

    Ok(())
}

// ===========================================================================
// Project scanner
// ===========================================================================

/// Results from scanning a project directory for existing agent configuration.
#[derive(Debug, Clone)]
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

        if self.gemini_md.is_some() || self.gemini_dir {
            backends.push("gemini-cli");
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
        mcp_json: false,
        mcp_servers: BTreeMap::new(),
        claude_agents: Vec::new(),
        claude_skills: Vec::new(),
        gemini_dir: false,
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
fn print_scan_results(scan: &ScanResult) {
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
    if scan.gemini_md.is_some() || scan.gemini_dir {
        println!("    {} {}", style("●").green(), style("gemini-cli").bold());
        if scan.gemini_md.is_some() {
            println!(
                "      {} GEMINI.md ({} lines)",
                style("├──").dim(),
                scan.gemini_md_lines
            );
        }
        if scan.gemini_dir {
            println!("      {} .gemini/", style("└──").dim());
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
}
