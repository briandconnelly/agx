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
    ("gemini-cli", "Gemini CLI (.gemini/, GEMINI.md)"),
    (
        "generic",
        "Generic AGENTS.md (Codex, Cursor, Windsurf)",
    ),
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
fn isolation_str(mode: &IsolationMode) -> &'static str {
    match mode {
        IsolationMode::Strict => "strict",
        IsolationMode::Permissive => "permissive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    #[test]
    fn builtin_backends_listed() {
        assert_eq!(BUILTIN_BACKENDS.len(), 3);
        assert_eq!(BUILTIN_BACKENDS[0].0, "claude-code");
        assert_eq!(BUILTIN_BACKENDS[1].0, "gemini-cli");
        assert_eq!(BUILTIN_BACKENDS[2].0, "generic");
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
}
