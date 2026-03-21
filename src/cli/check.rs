//! `agx check` — validate manifest, detect conflicts and incompatibilities.

use anyhow::Context;
use clap::Args;
use console::style;

use agx::config;
use agx::manifest;

#[derive(Debug, Args)]
pub struct CheckArgs {}

/// Run the `agx check` command.
pub fn run(_args: CheckArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    run_in(&cwd)
}

/// Check a manifest found at or above `start_dir`.
///
/// Separated from `run` so it can be tested without relying on the
/// process working directory.
pub fn run_in(start_dir: &std::path::Path) -> anyhow::Result<()> {
    let manifest_path = config::find_manifest(start_dir)
        .context("no agent-environment.yml found (run `agx init` first)")?;

    println!(
        "Checking {}...",
        style(manifest_path.display()).dim()
    );

    let env = manifest::load(&manifest_path)?;

    check_manifest(&env)
}

/// Validate a loaded manifest and print results.
///
/// Returns `Ok(())` for a valid manifest. Calls `std::process::exit(1)`
/// on validation errors (matching the expected CLI behavior).
pub fn check_manifest(
    env: &agx::manifest::types::AgentEnvironment,
) -> anyhow::Result<()> {
    match manifest::validate::validate(env) {
        Ok(()) => {
            println!(
                "{} Manifest is valid.",
                style("✓").green().bold()
            );
            Ok(())
        }
        Err(errors) => {
            for err in &errors {
                println!(
                    "  {} {}",
                    style("✗").red().bold(),
                    style(err).red()
                );
            }
            println!();
            println!(
                "{} Found {} error{}.",
                style("✗").red().bold(),
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn minimal_env() -> AgentEnvironment {
        AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: {
                let mut m = BTreeMap::new();
                m.insert(
                    "claude-code".to_string(),
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
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
            env_file: None,
        }
    }

    #[test]
    fn check_valid_manifest_succeeds() {
        let env = minimal_env();
        // check_manifest calls process::exit on error, but for valid manifests
        // it returns Ok(()).
        let result = check_manifest(&env);
        assert!(result.is_ok());
    }

    #[test]
    fn run_in_with_valid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("agent-environment.yml");
        std::fs::write(
            &manifest_path,
            "name: test-project\nbackends:\n  claude-code:\n    isolation: strict\n",
        )
        .unwrap();

        let result = run_in(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn run_in_no_manifest_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run_in(tmp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no agent-environment.yml found"));
    }

    #[test]
    fn run_in_with_invalid_yaml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("agent-environment.yml");
        std::fs::write(&manifest_path, "not: [valid: yaml: manifest").unwrap();

        let result = run_in(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn run_from_cwd_with_valid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: test-run\nbackends:\n  claude-code:\n    isolation: strict\n",
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = run(CheckArgs {});
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_from_cwd_no_manifest() {
        let tmp = tempfile::tempdir().unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = run(CheckArgs {});
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_err());
    }

    #[test]
    fn run_in_with_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: test-subdir\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        let sub = tmp.path().join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();

        let result = run_in(&sub);
        assert!(result.is_ok());
    }

    #[test]
    fn check_manifest_with_multiple_backends() {
        let mut env = minimal_env();
        env.backends.insert(
            "gemini-cli".to_string(),
            BackendConfig {
                default: None,
                isolation: None,
                model: None,
            },
        );
        let result = check_manifest(&env);
        assert!(result.is_ok());
    }

    #[test]
    fn check_manifest_with_registry_plugin() {
        let mut env = minimal_env();
        env.registries.insert(
            "official".to_string(),
            agx::manifest::types::RegistryConfig {
                url: "https://github.com/example/registry".to_string(),
                auth: None,
                pin: None,
            },
        );
        env.plugins.insert(
            "test-plugin".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=1.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let result = check_manifest(&env);
        assert!(result.is_ok());
    }

    #[test]
    fn check_manifest_with_agents() {
        let mut env = minimal_env();
        env.agents.insert(
            "helper".to_string(),
            agx::manifest::types::AgentDependency {
                git: None,
                path: "./agents/helper.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: Default::default(),
            },
        );
        let result = check_manifest(&env);
        assert!(result.is_ok());
    }

    #[test]
    fn check_manifest_with_plugins_and_skills() {
        let mut env = minimal_env();
        env.plugins.insert(
            "my-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: Some("https://github.com/org/repo.git".to_string()),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.skills.insert(
            "deploy".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/deploy".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let result = check_manifest(&env);
        assert!(result.is_ok());
    }
}
