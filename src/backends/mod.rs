//! Backend trait and implementations for per-framework code generation.

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod gemini_cli;
pub mod generic;
pub mod windsurf;

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::lockfile::types::Lockfile;
use crate::manifest::types::{AgentEnvironment, BackendConfig};

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
