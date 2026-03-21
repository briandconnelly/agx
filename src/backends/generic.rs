//! Generic backend — generates `AGENTS.md` for tools that support the standard.
//!
//! This backend writes `AGENTS.md` from the base context file only (no overlay),
//! since the file is inherently backend-agnostic. Skills are copied to a
//! `.agents/skills/` directory if applicable.

use super::{copy_skills, generate_agents_md, Backend, ResolvedEnvironment};
use crate::manifest::types::BackendConfig;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Generic AGENTS.md backend (Codex, Cursor, Windsurf, etc.).
///
/// Generates a minimal environment from the base context file:
/// - `AGENTS.md` — the project context (no backend-specific overlay)
/// - `.agents/skills/*/` — skill directories (if any are declared)
pub struct GenericBackend;

impl Backend for GenericBackend {
    fn name(&self) -> &str {
        "generic"
    }

    fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()> {
        let manifest = resolved.manifest;

        // Generate AGENTS.md from the base context file (no overlay).
        generate_agents_md(manifest.context.as_ref(), output_dir).context("generating AGENTS.md")?;

        // Copy skills to .agents/skills/ if any are declared.
        let skills_dir = output_dir.join(".agents").join("skills");
        copy_skills(&manifest.skills, resolved.cache_dir, output_dir, &skills_dir)
            .context("generating .agents/skills/")?;

        Ok(())
    }

    fn context_filename(&self) -> &str {
        "AGENTS.md"
    }

    fn run_command(&self, _config: &BackendConfig) -> Result<Command> {
        anyhow::bail!("generic backend does not support `agx run`")
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

        // Create a base context file.
        fs::write(dir.path().join("CONTEXT.md"), "# My Project\n\nThis is the project context.\n")
            .unwrap();

        let manifest: AgentEnvironment = serde_yaml::from_str(
            r#"
name: test-project
backends:
  generic: {}
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
        let backend = GenericBackend;
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
  generic: {}
"#,
        )
        .unwrap();

        let lockfile = Lockfile::new();
        let resolved = ResolvedEnvironment {
            manifest: &manifest,
            lockfile: &lockfile,
            cache_dir: dir.path(),
        };
        let backend = GenericBackend;
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
  generic: {}
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
        let backend = GenericBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("Base context file not found"));
    }

    #[test]
    fn test_generic_skill_copy() {
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
  generic: {}
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
        let backend = GenericBackend;
        backend.generate(&resolved, dir.path()).unwrap();

        let skill_md =
            fs::read_to_string(dir.path().join(".agents/skills/deploy/SKILL.md")).unwrap();
        assert!(skill_md.starts_with("<!-- AUTO-GENERATED BY agx."));
        assert!(skill_md.contains("# Deploy"));

        let helper =
            fs::read_to_string(dir.path().join(".agents/skills/deploy/helper.sh")).unwrap();
        assert!(helper.contains("echo deploy"));
    }

    #[test]
    fn test_generic_run_command_fails() {
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let backend = GenericBackend;
        let result = backend.run_command(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support"));
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
  generic: {}
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
        let backend = GenericBackend;

        backend.generate(&resolved, dir.path()).unwrap();
        let agents1 = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        let skill1 =
            fs::read_to_string(dir.path().join(".agents/skills/my-skill/SKILL.md")).unwrap();

        backend.generate(&resolved, dir.path()).unwrap();
        let agents2 = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        let skill2 =
            fs::read_to_string(dir.path().join(".agents/skills/my-skill/SKILL.md")).unwrap();

        assert_eq!(agents1, agents2);
        assert_eq!(skill1, skill2);
    }
}
