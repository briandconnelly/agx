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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // ---- get_backend tests ----

    #[test]
    fn get_backend_known_names() {
        for name in &["claude-code", "codex", "cursor", "gemini-cli", "generic", "windsurf"] {
            assert!(get_backend(name).is_some(), "should find backend: {}", name);
        }
    }

    #[test]
    fn get_backend_unknown_name() {
        assert!(get_backend("nonexistent").is_none());
        assert!(get_backend("").is_none());
    }

    // ---- inject_agent_frontmatter tests ----

    #[test]
    fn inject_frontmatter_no_changes() {
        let content = "# Hello World\n\nSome body text.";
        let result = inject_agent_frontmatter(content, None, &BTreeMap::new());
        assert_eq!(result, content);
    }

    #[test]
    fn inject_frontmatter_add_model_no_existing() {
        let content = "# Agent\n\nDo something.";
        let result = inject_agent_frontmatter(content, Some("opus"), &BTreeMap::new());
        assert!(result.contains("model: opus"));
        assert!(result.starts_with("---\n"));
        assert!(result.contains("# Agent"));
    }

    #[test]
    fn inject_frontmatter_add_model_with_existing() {
        let content = "---\nmodel: sonnet\ntimeout: 30\n---\n# Agent\n\nDo something.";
        let result = inject_agent_frontmatter(content, Some("opus"), &BTreeMap::new());
        assert!(result.contains("model: opus"));
        assert!(!result.contains("model: sonnet"));
        assert!(result.contains("timeout: 30"));
    }

    #[test]
    fn inject_frontmatter_add_params() {
        let content = "# Agent\n\nBody.";
        let mut params = BTreeMap::new();
        params.insert("timeout".to_string(), serde_yaml::Value::Number(serde_yaml::Number::from(60)));
        params.insert("verbose".to_string(), serde_yaml::Value::Bool(true));
        let result = inject_agent_frontmatter(content, None, &params);
        assert!(result.contains("timeout: 60"));
        assert!(result.contains("verbose: true"));
    }

    #[test]
    fn inject_frontmatter_override_existing_params() {
        let content = "---\ntimeout: 10\nlevel: debug\n---\n# Agent";
        let mut params = BTreeMap::new();
        params.insert("timeout".to_string(), serde_yaml::Value::String("30".to_string()));
        let result = inject_agent_frontmatter(content, None, &params);
        assert!(result.contains("timeout: 30"));
        assert!(result.contains("level: debug"));
        // Should not have duplicate timeout
        assert_eq!(result.matches("timeout").count(), 1);
    }

    #[test]
    fn inject_frontmatter_model_and_params() {
        let content = "---\nold: value\n---\n# Agent";
        let mut params = BTreeMap::new();
        params.insert("key".to_string(), serde_yaml::Value::String("val".to_string()));
        let result = inject_agent_frontmatter(content, Some("opus"), &params);
        assert!(result.contains("model: opus"));
        assert!(result.contains("key: val"));
        assert!(result.contains("old: value"));
    }

    // ---- parse_frontmatter tests ----

    #[test]
    fn parse_frontmatter_no_frontmatter() {
        let content = "# Hello\n\nBody text.";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn parse_frontmatter_valid() {
        let content = "---\nmodel: opus\n---\n# Agent\n\nBody.";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.unwrap(), "model: opus");
        assert_eq!(body, "# Agent\n\nBody.");
    }

    #[test]
    fn parse_frontmatter_malformed_no_closing() {
        let content = "---\nmodel: opus\n# Agent\n\nBody.";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    // ---- resolve_env_vars tests ----

    #[test]
    fn resolve_env_vars_no_vars() {
        let result = resolve_env_vars("hello world");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn resolve_env_vars_with_var() {
        unsafe { std::env::set_var("AGX_TEST_VAR_RESOLVE", "replaced"); }
        let result = resolve_env_vars("prefix-${AGX_TEST_VAR_RESOLVE}-suffix");
        assert_eq!(result, "prefix-replaced-suffix");
        unsafe { std::env::remove_var("AGX_TEST_VAR_RESOLVE"); }
    }

    #[test]
    fn resolve_env_vars_missing_var() {
        unsafe { std::env::remove_var("AGX_NONEXISTENT_VAR_12345"); }
        let result = resolve_env_vars("value=${AGX_NONEXISTENT_VAR_12345}!");
        assert_eq!(result, "value=!");
    }

    #[test]
    fn resolve_env_vars_multiple_vars() {
        unsafe { std::env::set_var("AGX_TEST_A", "aaa"); }
        unsafe { std::env::set_var("AGX_TEST_B", "bbb"); }
        let result = resolve_env_vars("${AGX_TEST_A}+${AGX_TEST_B}");
        assert_eq!(result, "aaa+bbb");
        unsafe { std::env::remove_var("AGX_TEST_A"); }
        unsafe { std::env::remove_var("AGX_TEST_B"); }
    }

    #[test]
    fn resolve_env_vars_dollar_without_brace() {
        let result = resolve_env_vars("$hello ${HOME}");
        // $hello should be left as-is (no brace), ${HOME} resolved
        assert!(result.starts_with("$hello "));
    }

    // ---- copy_dir_recursive tests ----

    #[test]
    fn copy_dir_recursive_basic() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        let dst = dst_dir.path().join("copied");

        fs::write(src_dir.path().join("file1.txt"), "hello").unwrap();
        fs::create_dir_all(src_dir.path().join("sub")).unwrap();
        fs::write(src_dir.path().join("sub/file2.txt"), "world").unwrap();

        copy_dir_recursive(src_dir.path(), &dst).unwrap();

        assert!(dst.join("file1.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("file1.txt")).unwrap(), "hello");
        assert!(dst.join("sub/file2.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("sub/file2.txt")).unwrap(), "world");
    }

    #[test]
    fn copy_dir_recursive_replaces_existing() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        let dst = dst_dir.path().join("dest");

        // Create initial destination
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("old.txt"), "old data").unwrap();

        // Create source
        fs::write(src_dir.path().join("new.txt"), "new data").unwrap();

        copy_dir_recursive(src_dir.path(), &dst).unwrap();

        assert!(!dst.join("old.txt").exists());
        assert!(dst.join("new.txt").exists());
    }

    // ---- generate_agents_md tests ----

    #[test]
    fn generate_agents_md_no_context() {
        let dir = tempfile::tempdir().unwrap();
        generate_agents_md(None, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(GENERATED_MARKER));
        assert!(content.contains("No context section defined"));
    }

    #[test]
    fn generate_agents_md_with_base_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CONTEXT.md"), "# My Project\n\nProject instructions here.\n").unwrap();

        let ctx = ContextConfig {
            base: "CONTEXT.md".to_string(),
            overlays: BTreeMap::new(),
            children: Vec::new(),
        };
        generate_agents_md(Some(&ctx), dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(GENERATED_MARKER));
        assert!(content.contains("# My Project"));
        assert!(content.contains("Project instructions here."));
    }

    #[test]
    fn generate_agents_md_missing_base_file() {
        let dir = tempfile::tempdir().unwrap();

        let ctx = ContextConfig {
            base: "nonexistent.md".to_string(),
            overlays: BTreeMap::new(),
            children: Vec::new(),
        };
        generate_agents_md(Some(&ctx), dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(GENERATED_MARKER));
        assert!(content.contains("not found"));
    }

    // ---- copy_skills tests ----

    #[test]
    fn copy_skills_empty() {
        let dir = tempfile::tempdir().unwrap();
        let skills: BTreeMap<String, SkillDependency> = BTreeMap::new();
        let result = copy_skills(&skills, dir.path(), dir.path(), &dir.path().join("target"));
        assert!(result.is_ok());
        assert!(!dir.path().join("target").exists());
    }

    #[test]
    fn copy_skills_local_skill() {
        let dir = tempfile::tempdir().unwrap();

        // Create a local skill directory with SKILL.md
        let skill_src = dir.path().join("skills/my-skill");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "# My Skill\n\nInstructions.").unwrap();
        fs::write(skill_src.join("helper.sh"), "#!/bin/bash\necho hi").unwrap();

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

        let target = dir.path().join("output/skills");
        copy_skills(&skills, dir.path(), dir.path(), &target).unwrap();

        assert!(target.join("my-skill/SKILL.md").exists());
        let content = fs::read_to_string(target.join("my-skill/SKILL.md")).unwrap();
        assert!(content.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(content.contains("local"));
        assert!(content.contains("# My Skill"));

        assert!(target.join("my-skill/helper.sh").exists());
    }

    #[test]
    fn copy_skills_local_skill_no_skill_md() {
        let dir = tempfile::tempdir().unwrap();

        // Create a local skill directory without SKILL.md
        let skill_src = dir.path().join("skills/no-md-skill");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("script.py"), "print('hello')").unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "no-md-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/no-md-skill".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        let target = dir.path().join("output/skills");
        copy_skills(&skills, dir.path(), dir.path(), &target).unwrap();

        assert!(target.join("no-md-skill/script.py").exists());
        assert!(!target.join("no-md-skill/SKILL.md").exists());
    }

    #[test]
    fn copy_skills_cached_skill() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        // Create a cached skill
        let cached_skill = cache_dir.join("skills/cached-skill");
        fs::create_dir_all(&cached_skill).unwrap();
        fs::write(cached_skill.join("SKILL.md"), "# Cached Skill\n\nRemote instructions.").unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "cached-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/someone/skills.git".to_string()),
                path: "skills/cached-skill".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
            },
        );

        let target = dir.path().join("output/skills");
        copy_skills(&skills, &cache_dir, dir.path(), &target).unwrap();

        assert!(target.join("cached-skill/SKILL.md").exists());
        let content = fs::read_to_string(target.join("cached-skill/SKILL.md")).unwrap();
        assert!(content.contains("cached cached-skill"));
    }

    #[test]
    fn copy_skills_marker_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();

        // Create a skill with SKILL.md that already has the marker
        let skill_src = dir.path().join("skills/pre-marked");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(
            skill_src.join("SKILL.md"),
            "<!-- AUTO-GENERATED BY agx. Source: local pre-marked -->\n# Already Marked",
        )
        .unwrap();

        let mut skills = BTreeMap::new();
        skills.insert(
            "pre-marked".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/pre-marked".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );

        let target = dir.path().join("output/skills");
        copy_skills(&skills, dir.path(), dir.path(), &target).unwrap();

        let content = fs::read_to_string(target.join("pre-marked/SKILL.md")).unwrap();
        // Marker should appear exactly once
        assert_eq!(content.matches("AUTO-GENERATED BY agx").count(), 1);
    }

    // ---- copy_agents tests ----

    #[test]
    fn copy_agents_empty() {
        let dir = tempfile::tempdir().unwrap();
        let agents: BTreeMap<String, AgentDependency> = BTreeMap::new();
        let result = copy_agents(&agents, dir.path(), dir.path(), &dir.path().join("target"));
        assert!(result.is_ok());
        assert!(!dir.path().join("target").exists());
    }

    #[test]
    fn copy_agents_local_agent() {
        let dir = tempfile::tempdir().unwrap();

        // Create a local agent file
        fs::create_dir_all(dir.path().join("agents")).unwrap();
        fs::write(
            dir.path().join("agents/my-helper.md"),
            "# My Helper\n\nDo helpful things.",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "my-helper".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/my-helper.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: Default::default(),
            },
        );

        let target = dir.path().join("output/agents");
        copy_agents(&agents, dir.path(), dir.path(), &target).unwrap();

        assert!(target.join("my-helper.md").exists());
        let content = fs::read_to_string(target.join("my-helper.md")).unwrap();
        assert!(content.contains("AUTO-GENERATED BY agx"));
        assert!(content.contains("model: opus"));
        assert!(content.contains("# My Helper"));
    }

    #[test]
    fn copy_agents_cached_agent() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");

        // Create a cached agent
        fs::create_dir_all(cache_dir.join("agents")).unwrap();
        fs::write(
            cache_dir.join("agents/remote-agent.md"),
            "# Remote Agent\n\nRemote instructions.",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "remote-agent".to_string(),
            AgentDependency {
                git: Some("https://github.com/someone/agents.git".to_string()),
                path: "agents/helper.md".to_string(),
                tag: Some("v2.0.0".to_string()),
                branch: None,
                commit: None,
                model: None,
                parameters: Default::default(),
            },
        );

        let target = dir.path().join("output/agents");
        copy_agents(&agents, &cache_dir, dir.path(), &target).unwrap();

        // With non-.md path ending, should use name.md
        assert!(target.join("helper.md").exists());
        let content = fs::read_to_string(target.join("helper.md")).unwrap();
        assert!(content.contains("AUTO-GENERATED BY agx"));
        assert!(content.contains("cached remote-agent"));
    }

    #[test]
    fn copy_agents_with_parameters() {
        let dir = tempfile::tempdir().unwrap();

        fs::create_dir_all(dir.path().join("agents")).unwrap();
        fs::write(
            dir.path().join("agents/param-agent.md"),
            "# Param Agent\n\nBody.",
        )
        .unwrap();

        let mut params = BTreeMap::new();
        params.insert("timeout".to_string(), serde_yaml::Value::Number(serde_yaml::Number::from(60)));

        let mut agents = BTreeMap::new();
        agents.insert(
            "param-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/param-agent.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("sonnet".to_string()),
                parameters: params,
            },
        );

        let target = dir.path().join("output/agents");
        copy_agents(&agents, dir.path(), dir.path(), &target).unwrap();

        let content = fs::read_to_string(target.join("param-agent.md")).unwrap();
        assert!(content.contains("model: sonnet"));
        assert!(content.contains("timeout: 60"));
    }

    #[test]
    fn copy_agents_non_md_path() {
        let dir = tempfile::tempdir().unwrap();

        fs::create_dir_all(dir.path().join("agents")).unwrap();
        fs::write(
            dir.path().join("agents/no-ext-agent"),
            "# No Extension Agent\n\nBody.",
        )
        .unwrap();

        let mut agents = BTreeMap::new();
        agents.insert(
            "no-ext-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/no-ext-agent".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: Default::default(),
            },
        );

        let target = dir.path().join("output/agents");
        copy_agents(&agents, dir.path(), dir.path(), &target).unwrap();

        // Path doesn't end with .md, so output should be name.md
        assert!(target.join("no-ext-agent.md").exists());
    }

    // ---- yaml_to_json tests ----

    #[test]
    fn yaml_to_json_null() {
        let result = yaml_to_json(&serde_yaml::Value::Null);
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn yaml_to_json_bool() {
        assert_eq!(yaml_to_json(&serde_yaml::Value::Bool(true)), json!(true));
        assert_eq!(yaml_to_json(&serde_yaml::Value::Bool(false)), json!(false));
    }

    #[test]
    fn yaml_to_json_integer() {
        let n = serde_yaml::Value::Number(serde_yaml::Number::from(42));
        assert_eq!(yaml_to_json(&n), json!(42));
    }

    #[test]
    fn yaml_to_json_float() {
        let n: serde_yaml::Value = serde_yaml::from_str("3.14").unwrap();
        let result = yaml_to_json(&n);
        assert!(result.as_f64().is_some());
    }

    #[test]
    fn yaml_to_json_string() {
        let s = serde_yaml::Value::String("hello".to_string());
        assert_eq!(yaml_to_json(&s), json!("hello"));
    }

    #[test]
    fn yaml_to_json_sequence() {
        let seq: serde_yaml::Value = serde_yaml::from_str("[1, 2, 3]").unwrap();
        let result = yaml_to_json(&seq);
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn yaml_to_json_mapping() {
        let map: serde_yaml::Value = serde_yaml::from_str("key: value\nnum: 42").unwrap();
        let result = yaml_to_json(&map);
        assert_eq!(result["key"], json!("value"));
        assert_eq!(result["num"], json!(42));
    }

    #[test]
    fn yaml_to_json_nested() {
        let nested: serde_yaml::Value = serde_yaml::from_str(
            "outer:\n  inner: [1, true, null]\n  name: test",
        )
        .unwrap();
        let result = yaml_to_json(&nested);
        assert_eq!(result["outer"]["inner"], json!([1, true, null]));
        assert_eq!(result["outer"]["name"], json!("test"));
    }

    #[test]
    fn yaml_to_json_mapping_with_non_string_keys() {
        // Create a mapping with an integer key
        let yaml: serde_yaml::Value = serde_yaml::from_str("42: value").unwrap();
        let result = yaml_to_json(&yaml);
        assert!(result.is_object());
        // The key 42 should be converted to string
        assert!(result.as_object().unwrap().contains_key("42"));
    }

    // ---- GENERATED_MARKER test ----

    #[test]
    fn generated_marker_is_consistent() {
        assert!(GENERATED_MARKER.contains("agx"));
        assert!(GENERATED_MARKER.contains("DO NOT EDIT"));
    }
}
