//! Codex backend — generates `.codex/`, `AGENTS.md`.
//!
//! Codex uses `AGENTS.md` as its context file and stores skills in `.codex/skills/`.
//! It supports `agx run` by launching the `codex` CLI.

use super::{copy_skills, generate_agents_md, Backend, ResolvedEnvironment};
use crate::manifest::types::{BackendConfig, IsolationMode, ModelSpec};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Codex backend implementation.
///
/// Generates the Codex environment from a resolved manifest:
/// - `.codex/skills/*/` — skill directories
/// - `AGENTS.md` — project context from base context file
pub struct CodexBackend;

impl Backend for CodexBackend {
    fn name(&self) -> &str {
        "codex"
    }

    fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()> {
        let manifest = resolved.manifest;

        // Generate AGENTS.md from the base context file.
        generate_agents_md(manifest.context.as_ref(), output_dir).context("generating AGENTS.md")?;

        // Copy skills to .codex/skills/ if any are declared.
        let skills_dir = output_dir.join(".codex").join("skills");
        copy_skills(&manifest.skills, resolved.cache_dir, output_dir, &skills_dir)
            .context("generating .codex/skills/")?;

        Ok(())
    }

    fn context_filename(&self) -> &str {
        "AGENTS.md"
    }

    fn run_command(&self, config: &BackendConfig) -> Result<Command> {
        let mut cmd = Command::new("codex");

        let isolation = config.isolation.clone().unwrap_or_default();
        match isolation {
            IsolationMode::Bare => {
                // Maximum isolation: read-only sandbox, never ask for approval.
                cmd.args(["--sandbox", "read-only"]);
                cmd.args(["--ask-for-approval", "never"]);
            }
            IsolationMode::Strict => {
                // Restrict writes to workspace only.
                cmd.args(["--sandbox", "read-only"]);
            }
            IsolationMode::Permissive => {
                // Allow writes within the workspace.
                cmd.args(["--sandbox", "workspace-write"]);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ResolvedEnvironment;
    use crate::lockfile::types::Lockfile;
    use crate::manifest::types::AgentEnvironment;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_agents_md_with_base_context() {
        let dir = tempdir().unwrap();

        fs::write(
            dir.path().join("CONTEXT.md"),
            "# My Project\n\nThis is the project context.\n",
        )
        .unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  codex: {}
context:
  base: ./CONTEXT.md
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: dir.path(),
        };
        let backend = CodexBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("DO NOT EDIT"));
        assert!(content.contains("# My Project"));
        assert!(content.contains("This is the project context."));
    }

    #[test]
    fn test_agents_md_no_context_section() {
        let dir = tempdir().unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  codex: {}
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: dir.path(),
        };
        let backend = CodexBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("DO NOT EDIT"));
        assert!(content.contains("No context section defined"));
    }

    #[test]
    fn test_agents_md_base_file_missing() {
        let dir = tempdir().unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  codex: {}
context:
  base: ./CONTEXT.md
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: dir.path(),
        };
        let backend = CodexBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("Base context file not found"));
    }

    #[test]
    fn test_codex_skill_copy() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        let skill_src = dir.path().join("skills").join("deploy");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "# Deploy\n").unwrap();
        fs::write(skill_src.join("helper.sh"), "#!/bin/bash\necho deploy").unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  codex: {}
skills:
  deploy:
    path: ./skills/deploy
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: cache.path(),
        };
        let backend = CodexBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        // Skills go to .codex/skills/ (not .agents/skills/).
        let skill_md =
            fs::read_to_string(dir.path().join(".codex/skills/deploy/SKILL.md")).unwrap();
        assert!(skill_md.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(skill_md.contains("# Deploy"));

        let helper =
            fs::read_to_string(dir.path().join(".codex/skills/deploy/helper.sh")).unwrap();
        assert!(helper.contains("echo deploy"));
    }

    #[test]
    fn test_codex_run_command_with_model() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: Some(ModelSpec::Simple("o3".to_string())),
        };
        let backend = CodexBackend;
        let cmd = backend.run_command(&config).unwrap();

        assert_eq!(cmd.get_program(), "codex");
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--model")));
        assert!(args.contains(&&std::ffi::OsStr::new("o3")));
    }

    #[test]
    fn test_codex_run_command_strict() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        let backend = CodexBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--sandbox")));
        assert!(args.contains(&&std::ffi::OsStr::new("read-only")));
        assert!(!args.contains(&&std::ffi::OsStr::new("--ask-for-approval")));
    }

    #[test]
    fn test_codex_run_command_bare() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Bare),
            model: None,
        };
        let backend = CodexBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--sandbox")));
        assert!(args.contains(&&std::ffi::OsStr::new("read-only")));
        assert!(args.contains(&&std::ffi::OsStr::new("--ask-for-approval")));
        assert!(args.contains(&&std::ffi::OsStr::new("never")));
    }

    #[test]
    fn test_codex_run_command_permissive() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let backend = CodexBackend;
        let cmd = backend.run_command(&config).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--sandbox")));
        assert!(args.contains(&&std::ffi::OsStr::new("workspace-write")));
    }

    #[test]
    fn test_generate_idempotent() {
        let dir = tempdir().unwrap();
        let cache = tempdir().unwrap();

        fs::write(dir.path().join("CONTEXT.md"), "# Project\n").unwrap();

        let skill_src = dir.path().join("skills").join("my-skill");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "Skill content\n").unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  codex: {}
context:
  base: ./CONTEXT.md
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
        let backend = CodexBackend;

        backend.generate(&resolved, dir.path()).unwrap();
        let agents1 = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        let skill1 =
            fs::read_to_string(dir.path().join(".codex/skills/my-skill/SKILL.md")).unwrap();

        backend.generate(&resolved, dir.path()).unwrap();
        let agents2 = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        let skill2 =
            fs::read_to_string(dir.path().join(".codex/skills/my-skill/SKILL.md")).unwrap();

        assert_eq!(agents1, agents2);
        assert_eq!(skill1, skill2);
    }

    #[test]
    fn test_name_and_context_filename() {
        let backend = CodexBackend;
        assert_eq!(backend.name(), "codex");
        assert_eq!(backend.context_filename(), "AGENTS.md");
    }
}
