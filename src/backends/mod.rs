//! Backend trait and implementations for per-framework code generation.

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod gemini_cli;
pub mod generic;
pub mod windsurf;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::lockfile::types::Lockfile;
use crate::manifest::types::{
    AgentDependency, AgentEnvironment, BackendConfig, ContextConfig, SkillDependency,
};

/// Auto-generated marker placed in JSON and Markdown files.
pub const GENERATED_MARKER: &str =
    "agx \u{2014} DO NOT EDIT. Update agent-environment.yml and run: agx generate";

/// The resolved environment that a backend uses to generate output.
/// Combines manifest declarations with pinned lockfile data.
pub struct ResolvedEnvironment<'a> {
    /// The parsed manifest.
    pub manifest: &'a AgentEnvironment,
    /// The resolved lockfile.
    pub lockfile: &'a Lockfile,
    /// Path to the global cache directory.
    pub cache_dir: &'a Path,
}

/// Trait that all backend modules must implement.
pub trait Backend {
    /// Backend identifier (e.g. "claude-code", "gemini-cli").
    fn name(&self) -> &str;

    /// Generate all output files for this backend.
    fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()>;

    /// The filename for the generated context file (e.g. "CLAUDE.md", "GEMINI.md").
    fn context_filename(&self) -> &str;

    /// Build the command to launch this backend's agent framework.
    fn run_command(&self, config: &BackendConfig) -> Result<Command>;
}

/// Look up a backend implementation by name.
pub fn get_backend(name: &str) -> Option<Box<dyn Backend>> {
    match name {
        "claude-code" => Some(Box::new(claude_code::ClaudeCodeBackend)),
        "codex" => Some(Box::new(codex::CodexBackend)),
        "cursor" => Some(Box::new(cursor::CursorBackend)),
        "gemini-cli" => Some(Box::new(gemini_cli::GeminiCliBackend)),
        "generic" => Some(Box::new(generic::GenericBackend)),
        "windsurf" => Some(Box::new(windsurf::WindsurfBackend)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared utility functions used by multiple backends
// ---------------------------------------------------------------------------

/// Inject or override YAML frontmatter in an agent Markdown file.
///
/// If `model` is specified, it is added to (or overrides) the frontmatter.
/// Any additional `params` are also merged into the frontmatter.
/// If no frontmatter exists, a new block is created.
pub fn inject_agent_frontmatter(
    content: &str,
    model: Option<&str>,
    params: &BTreeMap<String, serde_yaml::Value>,
) -> String {
    if model.is_none() && params.is_empty() {
        return content.to_string();
    }

    let (existing_frontmatter, body) = parse_frontmatter(content);

    let mut fm_lines: Vec<String> = Vec::new();

    // Start with existing frontmatter lines (excluding model if we're overriding).
    if let Some(ref existing) = existing_frontmatter {
        for line in existing.lines() {
            // Skip existing model line if we're overriding.
            if model.is_some() && line.trim_start().starts_with("model:") {
                continue;
            }
            // Skip existing param lines if we're overriding.
            let key = line.split(':').next().unwrap_or("").trim();
            if !key.is_empty() && params.contains_key(key) {
                continue;
            }
            fm_lines.push(line.to_string());
        }
    }

    // Add model.
    if let Some(m) = model {
        fm_lines.push(format!("model: {m}"));
    }

    // Add parameters.
    for (k, v) in params {
        let val_str = match v {
            serde_yaml::Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other)
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        fm_lines.push(format!("{k}: {val_str}"));
    }

    let frontmatter = fm_lines.join("\n");
    format!("---\n{frontmatter}\n---\n{body}")
}

/// Parse YAML frontmatter from Markdown content.
/// Returns (frontmatter_content, body) where frontmatter_content is None if
/// no frontmatter block was found.
pub fn parse_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing ---
    let after_opening = &trimmed[3..];
    let after_opening = after_opening.strip_prefix('\n').unwrap_or(after_opening);

    if let Some(end_idx) = after_opening.find("\n---") {
        let fm = after_opening[..end_idx].to_string();
        let rest = &after_opening[end_idx + 4..]; // skip \n---
        let body = rest.strip_prefix('\n').unwrap_or(rest);
        (Some(fm), body.to_string())
    } else {
        // Malformed frontmatter — no closing delimiter. Treat whole thing as body.
        (None, content.to_string())
    }
}

/// Resolve `${VAR_NAME}` references in a string from the current shell environment.
/// Unresolved variables are left as empty strings.
pub fn resolve_env_vars(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            for ch in chars.by_ref() {
                if ch == '}' {
                    break;
                }
                var_name.push(ch);
            }
            result.push_str(&std::env::var(&var_name).unwrap_or_default());
        } else {
            result.push(c);
        }
    }

    result
}

/// Recursively copy a directory tree, replacing the destination if it exists.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    fs::create_dir_all(dst)?;

    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(src)?;
        let target = dst.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

/// Generate `AGENTS.md` from the base context file.
///
/// If a context section with a `base` path is declared in the manifest, reads
/// that file and writes it as `AGENTS.md` with the agx marker prepended. If no
/// context section exists, writes a minimal placeholder.
///
/// Used by generic, codex, cursor, and windsurf backends.
pub fn generate_agents_md(
    context: Option<&ContextConfig>,
    output_dir: &Path,
) -> Result<()> {
    let marker = format!("<!-- {GENERATED_MARKER} -->\n");

    let content = if let Some(ctx) = context {
        let base_path = output_dir.join(&ctx.base);
        if base_path.exists() {
            let base_content = fs::read_to_string(&base_path)
                .with_context(|| format!("reading base context: {}", base_path.display()))?;
            format!("{marker}{base_content}")
        } else {
            format!(
                "{marker}# Project Context\n\nBase context file not found: {}\n",
                ctx.base
            )
        }
    } else {
        format!("{marker}# Project Context\n\nNo context section defined in agent-environment.yml.\n")
    };

    fs::write(output_dir.join("AGENTS.md"), content)?;

    Ok(())
}

/// Copy skill directories to a target directory, adding agx markers to SKILL.md files.
///
/// This is the shared skill-copying logic used by all backends. Each backend
/// passes its own target directory (e.g. `.claude/skills/`, `.codex/skills/`,
/// `.cursor/rules/`, `.windsurf/rules/`, `.agents/skills/`).
pub fn copy_skills(
    skills: &BTreeMap<String, SkillDependency>,
    cache_dir: &Path,
    output_dir: &Path,
    target_dir: &Path,
) -> Result<()> {
    if skills.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(target_dir)
        .with_context(|| format!("creating {} directory", target_dir.display()))?;

    for (name, skill) in skills {
        let source_dir = if skill.is_local() {
            output_dir.join(&skill.path)
        } else {
            cache_dir.join("skills").join(name)
        };

        let dest_dir = target_dir.join(name);
        copy_dir_recursive(&source_dir, &dest_dir)
            .with_context(|| format!("copying skill '{name}' from {}", source_dir.display()))?;

        // Add marker to SKILL.md if it exists.
        let skill_md = dest_dir.join("SKILL.md");
        if skill_md.exists() {
            let content = fs::read_to_string(&skill_md)?;
            let source_label = if skill.is_local() {
                format!("local {}", skill.path)
            } else {
                format!("cached {name}")
            };
            let marker = format!("<!-- AUTO-GENERATED BY agx. Source: {source_label} -->\n");
            // Only add marker if not already present.
            if !content.starts_with("<!-- AUTO-GENERATED BY agx.") {
                fs::write(&skill_md, format!("{marker}{content}"))?;
            }
        }
    }

    Ok(())
}

/// Copy agent files to a target directory, injecting YAML frontmatter with
/// model/parameter overrides and adding agx markers.
///
/// This is the shared agent-copying logic used by backends that place agents
/// in a framework-specific directory (e.g. `.claude/agents/`, `.gemini/agents/`).
pub fn copy_agents(
    agents: &BTreeMap<String, AgentDependency>,
    cache_dir: &Path,
    output_dir: &Path,
    agents_dir: &Path,
) -> Result<()> {
    if agents.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(agents_dir)
        .with_context(|| format!("creating {} directory", agents_dir.display()))?;

    for (name, agent) in agents {
        let source_path = if agent.is_local() {
            output_dir.join(&agent.path)
        } else {
            // Cached agent: look in cache_dir/agents/<name>.md
            cache_dir.join("agents").join(format!("{name}.md"))
        };

        let content = fs::read_to_string(&source_path)
            .with_context(|| format!("reading agent source: {}", source_path.display()))?;

        let injected =
            inject_agent_frontmatter(&content, agent.model.as_deref(), &agent.parameters);
        let source_label = if agent.is_local() {
            format!("local {}", agent.path)
        } else {
            format!("cached {name}")
        };
        let marker = format!("<!-- AUTO-GENERATED BY agx. Source: {source_label} -->\n");

        let output_filename = if agent.path.ends_with(".md") {
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

/// Convert a `serde_yaml::Value` to a `serde_json::Value`.
pub fn yaml_to_json(yaml: &serde_yaml::Value) -> Value {
    match yaml {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                json!(i)
            } else if let Some(f) = n.as_f64() {
                json!(f)
            } else {
                Value::Null
            }
        }
        serde_yaml::Value::String(s) => json!(s),
        serde_yaml::Value::Sequence(seq) => {
            Value::Array(seq.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = Map::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => serde_yaml::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                obj.insert(key, yaml_to_json(v));
            }
            Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}
