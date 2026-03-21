//! `agx backends` — list available backends.

use anyhow::Result;
use clap::Args;
use console::style;

use agx::config;
use agx::manifest;
use agx::manifest::types::IsolationMode;

#[derive(Debug, Args)]
pub struct BackendsArgs {}

/// Built-in backend descriptions.
const BUILTIN_BACKENDS: &[(&str, &str)] = &[
    (
        "claude-code",
        "Claude Code (.claude/, .mcp.json, CLAUDE.md)",
    ),
    ("codex", "Codex (.codex/, AGENTS.md)"),
    ("cursor", "Cursor (.cursor/rules/, AGENTS.md)"),
    ("gemini-cli", "Gemini CLI (.gemini/, GEMINI.md)"),
    (
        "generic",
        "Generic AGENTS.md (Windsurf, etc.)",
    ),
    ("windsurf", "Windsurf (Codeium)"),
];

pub fn run(_args: BackendsArgs) -> Result<()> {
    // Always show built-in backends
    println!("{}", style("Built-in backends:").bold());
    for (name, desc) in BUILTIN_BACKENDS {
        println!(
            "  {:<15}{}",
            style(name).cyan(),
            style(desc).dim()
        );
    }

    // Try to load manifest to show project backends
    let cwd = std::env::current_dir().ok();
    let manifest_path = cwd.as_deref().and_then(|d| config::find_manifest(d));

    if let Some(path) = manifest_path {
        if let Ok(env) = manifest::load(&path) {
            if !env.backends.is_empty() {
                println!();
                println!(
                    "{} (from agent-environment.yml):",
                    style("Project backends").bold()
                );
                for (name, config) in &env.backends {
                    let mut annotations = Vec::new();

                    if config.default == Some(true) {
                        annotations.push("default".to_string());
                    }

                    let isolation = config
                        .isolation
                        .as_ref()
                        .unwrap_or(&IsolationMode::Strict);
                    annotations.push(format!("{}", isolation_str(isolation)));

                    let suffix = if annotations.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", annotations.join(", "))
                    };

                    println!(
                        "  {} {}{}",
                        style("●").green(),
                        style(name).bold().cyan(),
                        style(suffix).dim()
                    );
                }
            }
        }
    }

    Ok(())
}

/// Convert isolation mode to display string.
fn isolation_str(mode: &IsolationMode) -> String {
    mode.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    #[test]
    fn builtin_backends_listed() {
        assert_eq!(BUILTIN_BACKENDS.len(), 6);
        assert_eq!(BUILTIN_BACKENDS[0].0, "claude-code");
        assert_eq!(BUILTIN_BACKENDS[1].0, "codex");
        assert_eq!(BUILTIN_BACKENDS[2].0, "cursor");
        assert_eq!(BUILTIN_BACKENDS[3].0, "gemini-cli");
        assert_eq!(BUILTIN_BACKENDS[4].0, "generic");
        assert_eq!(BUILTIN_BACKENDS[5].0, "windsurf");
    }

    #[test]
    fn isolation_str_strict() {
        assert_eq!(isolation_str(&IsolationMode::Strict), "strict");
    }

    #[test]
    fn isolation_str_permissive() {
        assert_eq!(isolation_str(&IsolationMode::Permissive), "permissive");
    }

    #[test]
    fn manifest_with_backends_shows_project_info() {
        let yaml = r#"
name: test-project
backends:
  claude-code:
    default: true
    isolation: strict
  gemini-cli:
    isolation: permissive
"#;
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(env.backends.len(), 2);

        let cc = &env.backends["claude-code"];
        assert_eq!(cc.default, Some(true));
        assert_eq!(cc.isolation, Some(IsolationMode::Strict));

        let gc = &env.backends["gemini-cli"];
        assert_eq!(gc.default, None);
        assert_eq!(gc.isolation, Some(IsolationMode::Permissive));
    }

    #[test]
    fn manifest_without_backends() {
        let yaml = "name: minimal\nbackends: {}\n";
        let env: AgentEnvironment = serde_yaml::from_str(yaml).unwrap();
        assert!(env.backends.is_empty());
    }

    #[test]
    fn run_without_manifest_succeeds() {
        // The `run` function should succeed even when there is no manifest
        // in the current directory — it always prints built-in backends.
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(BackendsArgs {});
        std::env::set_current_dir(&old_cwd).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_manifest_containing_backends() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("agent-environment.yml");
        std::fs::write(
            &manifest_path,
            r#"
name: test-run
backends:
  claude-code:
    default: true
    isolation: strict
  gemini-cli:
    isolation: permissive
"#,
        )
        .unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(BackendsArgs {});
        std::env::set_current_dir(&old_cwd).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_manifest_empty_backends() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("agent-environment.yml");
        std::fs::write(&manifest_path, "name: empty-be\nbackends: {}\n").unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = run(BackendsArgs {});
        std::env::set_current_dir(&old_cwd).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn builtin_backend_descriptions_are_nonempty() {
        for (name, desc) in BUILTIN_BACKENDS {
            assert!(!name.is_empty(), "backend name should not be empty");
            assert!(!desc.is_empty(), "backend description should not be empty");
        }
    }

    #[test]
    fn manifest_backend_default_none_uses_strict_display() {
        // When default is None and isolation is None, the display code
        // defaults to Strict for the isolation string.
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let isolation = config
            .isolation
            .as_ref()
            .unwrap_or(&IsolationMode::Strict);
        assert_eq!(isolation_str(isolation), "strict");
    }

    #[test]
    fn manifest_backend_default_true_annotation() {
        let config = BackendConfig {
            default: Some(true),
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let mut annotations = Vec::new();
        if config.default == Some(true) {
            annotations.push("default".to_string());
        }
        let isolation = config
            .isolation
            .as_ref()
            .unwrap_or(&IsolationMode::Strict);
        annotations.push(isolation_str(isolation).to_string());

        assert_eq!(annotations, vec!["default", "permissive"]);
        let suffix = format!(" ({})", annotations.join(", "));
        assert_eq!(suffix, " (default, permissive)");
    }

    #[test]
    fn manifest_backend_no_default_annotation() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        let mut annotations = Vec::new();
        if config.default == Some(true) {
            annotations.push("default".to_string());
        }
        let isolation = config
            .isolation
            .as_ref()
            .unwrap_or(&IsolationMode::Strict);
        annotations.push(isolation_str(isolation).to_string());

        // No "default" annotation; just "strict"
        assert_eq!(annotations, vec!["strict"]);
    }
}
