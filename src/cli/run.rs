//! `agx run` — launch a backend in the managed environment.

use anyhow::{bail, Context};
use clap::Args;
use console::style;

use agx::backends::get_backend;
use agx::config;
use agx::lockfile;
use agx::manifest::types::AgentEnvironment;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Backend to launch (e.g. "claude-code", "gemini-cli")
    pub backend: Option<String>,

    /// Model override for this session
    #[arg(long)]
    pub model: Option<String>,

    /// Effort level override
    #[arg(long)]
    pub effort: Option<String>,

    /// Arguments to pass through to the backend
    #[arg(last = true)]
    pub backend_args: Vec<String>,
}

/// Run the `agx run` command.
///
/// Performs a pre-flight check (manifest exists, staleness detection), selects
/// the appropriate backend, builds the launch command with any runtime
/// overrides, and execs the backend process.
pub fn run(args: RunArgs) -> anyhow::Result<()> {
    let ctx = super::ProjectContext::load()?;
    let manifest = &ctx.manifest;
    let manifest_path = &ctx.manifest_path;
    let project_root = &ctx.project_root;

    // 2. Staleness detection.
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    if !lockfile_path.exists() {
        eprintln!(
            "{} lockfile not found; run {} to resolve dependencies",
            style("warning:").yellow().bold(),
            style("agx install").cyan(),
        );
    } else {
        let stale = check_staleness(manifest_path, &lockfile_path, manifest)?;
        if stale {
            eprintln!(
                "{} environment may be out of date; run {} to update",
                style("warning:").yellow().bold(),
                style("agx install").cyan(),
            );
        }
    }

    // 3. Determine backend.
    let backend_name = determine_backend(&args.backend, manifest)?;

    // 4. Look up backend implementation.
    let backend = get_backend(&backend_name).ok_or_else(|| {
        anyhow::anyhow!(
            "no built-in backend for '{}'; available: claude-code, gemini-cli, generic",
            backend_name
        )
    })?;

    // 5. Build launch command.
    let backend_config = manifest.backends.get(&backend_name).ok_or_else(|| {
        anyhow::anyhow!("backend '{}' not found in manifest backends", backend_name)
    })?;
    let mut cmd = backend.run_command(backend_config)?;

    // 6. Apply runtime overrides.
    if let Some(ref model) = args.model {
        cmd.args(["--model", model]);
    }
    if let Some(ref effort) = args.effort {
        cmd.args(["--effort", effort]);
    }

    // 7. Forward backend args.
    for arg in &args.backend_args {
        cmd.arg(arg);
    }

    // 8. Launch — exec the command, forward exit code.
    let status = cmd.status().with_context(|| {
        format!(
            "failed to launch backend '{}' (is '{}' installed?)",
            backend_name,
            cmd.get_program().to_string_lossy()
        )
    })?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Determine which backend to use.
///
/// If `specified` is given, validates it exists in the manifest.
/// Otherwise, finds the default backend: the one with `default: true`,
/// or the first listed backend.
pub fn determine_backend(
    specified: &Option<String>,
    manifest: &AgentEnvironment,
) -> anyhow::Result<String> {
    if let Some(name) = specified {
        if !manifest.backends.contains_key(name) {
            bail!(
                "backend '{}' is not declared in the manifest; available: {}",
                name,
                manifest
                    .backends
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(name.clone());
    }

    // Look for a backend with default: true.
    for (name, config) in &manifest.backends {
        if config.default == Some(true) {
            return Ok(name.clone());
        }
    }

    // Fall back to first listed backend.
    manifest
        .backends
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no backends declared in manifest"))
}

/// Check whether the environment is stale.
///
/// Returns `true` if the manifest is newer than the lockfile (by mtime),
/// or if the lockfile content is stale relative to the manifest entries.
fn check_staleness(
    manifest_path: &std::path::Path,
    lockfile_path: &std::path::Path,
    manifest: &AgentEnvironment,
) -> anyhow::Result<bool> {
    // Check mtime: manifest newer than lockfile.
    let manifest_mtime = std::fs::metadata(manifest_path)
        .and_then(|m| m.modified())
        .ok();
    let lockfile_mtime = std::fs::metadata(lockfile_path)
        .and_then(|m| m.modified())
        .ok();

    if let (Some(m_time), Some(l_time)) = (manifest_mtime, lockfile_mtime) {
        if m_time > l_time {
            return Ok(true);
        }
    }

    // Check content: lockfile entries vs manifest entries.
    let lf = lockfile::load(lockfile_path)?;
    Ok(lockfile::is_stale(&lf, manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::{AgentEnvironment, BackendConfig, IsolationMode, ModelSpec};
    use std::collections::BTreeMap;

    /// Helper: create a manifest with the given backends.
    fn manifest_with_backends(backends: Vec<(&str, Option<bool>)>) -> AgentEnvironment {
        let mut backend_map = BTreeMap::new();
        for (name, default) in backends {
            backend_map.insert(
                name.to_string(),
                BackendConfig {
                    default,
                    isolation: Some(IsolationMode::Strict),
                    model: None,
                },
            );
        }
        AgentEnvironment {
            name: "test".to_string(),
            version: None,
            description: None,
            backends: backend_map,
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        }
    }

    #[test]
    fn backend_selection_explicit() {
        let manifest =
            manifest_with_backends(vec![("claude-code", Some(true)), ("gemini-cli", None)]);
        let result = determine_backend(&Some("gemini-cli".to_string()), &manifest).unwrap();
        assert_eq!(result, "gemini-cli");
    }

    #[test]
    fn backend_selection_explicit_not_declared() {
        let manifest = manifest_with_backends(vec![("claude-code", None)]);
        let err = determine_backend(&Some("nonexistent".to_string()), &manifest);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("nonexistent"));
        assert!(msg.contains("not declared"));
    }

    #[test]
    fn backend_selection_default_true() {
        let manifest =
            manifest_with_backends(vec![("gemini-cli", None), ("claude-code", Some(true))]);
        let result = determine_backend(&None, &manifest).unwrap();
        assert_eq!(result, "claude-code");
    }

    #[test]
    fn backend_selection_first_listed() {
        // BTreeMap is sorted, so "claude-code" comes before "gemini-cli" alphabetically.
        let manifest = manifest_with_backends(vec![("gemini-cli", None), ("claude-code", None)]);
        let result = determine_backend(&None, &manifest).unwrap();
        assert_eq!(result, "claude-code");
    }

    #[test]
    fn backend_selection_no_backends() {
        let manifest = manifest_with_backends(vec![]);
        let err = determine_backend(&None, &manifest);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("no backends"));
    }

    #[test]
    fn runtime_model_override_appended() {
        let config = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Simple("sonnet".to_string())),
        };
        let backend = agx::backends::claude_code::ClaudeCodeBackend;
        let mut cmd =
            <agx::backends::claude_code::ClaudeCodeBackend as agx::backends::Backend>::run_command(
                &backend, &config,
            )
            .unwrap();

        // Apply --model override (simulating what run() does).
        cmd.args(["--model", "opus"]);

        let args: Vec<_> = cmd.get_args().collect();
        // The config-based model (sonnet) and the override (opus) are both present.
        // The last --model wins when the CLI processes them.
        let model_args: Vec<_> = args
            .windows(2)
            .filter(|w| w[0] == "--model")
            .map(|w| w[1].to_string_lossy().to_string())
            .collect();
        assert!(model_args.contains(&"sonnet".to_string()));
        assert!(model_args.contains(&"opus".to_string()));
    }

    #[test]
    fn runtime_effort_override_appended() {
        let config = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let backend = agx::backends::claude_code::ClaudeCodeBackend;
        let mut cmd =
            <agx::backends::claude_code::ClaudeCodeBackend as agx::backends::Backend>::run_command(
                &backend, &config,
            )
            .unwrap();

        cmd.args(["--effort", "high"]);

        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&&std::ffi::OsStr::new("--effort")));
        assert!(args.contains(&&std::ffi::OsStr::new("high")));
    }

    #[test]
    fn staleness_detected_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("agent-environment.yml");
        let lockfile_path = dir.path().join("agent-environment.lock");

        // Write lockfile first so it is older.
        let lockfile = agx::lockfile::types::Lockfile::new();
        agx::lockfile::save(&lockfile, &lockfile_path).unwrap();

        // Write manifest with a skill (not in lockfile).
        let manifest_yaml = r#"
name: test
backends:
  claude-code: {}
skills:
  my-skill:
    path: ./skills/my-skill
"#;
        std::fs::write(&manifest_path, manifest_yaml).unwrap();
        let manifest: AgentEnvironment = serde_yaml::from_str(manifest_yaml).unwrap();

        // Content check should detect staleness even if mtime is not an issue.
        let lf = agx::lockfile::load(&lockfile_path).unwrap();
        assert!(agx::lockfile::is_stale(&lf, &manifest));
    }

    #[test]
    fn not_stale_when_in_sync() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("agent-environment.yml");
        let lockfile_path = dir.path().join("agent-environment.lock");

        let manifest_yaml = "name: test\nbackends:\n  claude-code: {}\n";
        // Write manifest first.
        std::fs::write(&manifest_path, manifest_yaml).unwrap();
        let manifest: AgentEnvironment = serde_yaml::from_str(manifest_yaml).unwrap();

        // Write lockfile after manifest (so it is newer by mtime).
        let lockfile = agx::lockfile::types::Lockfile::new();
        agx::lockfile::save(&lockfile, &lockfile_path).unwrap();

        assert!(!check_staleness(&manifest_path, &lockfile_path, &manifest).unwrap());
    }

    #[test]
    fn exit_code_forwarding_success() {
        let status = std::process::Command::new("true").status().unwrap();
        assert_eq!(status.code(), Some(0));
    }

    #[test]
    fn exit_code_forwarding_failure() {
        let status = std::process::Command::new("false").status().unwrap();
        assert_eq!(status.code(), Some(1));
    }
}
