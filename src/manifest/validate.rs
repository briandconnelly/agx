//! Manifest validation logic.
//!
//! Validates structural correctness of a parsed `AgentEnvironment` manifest,
//! including version constraint syntax, path references, registry references,
//! MCP server configuration, and context file requirements.

use super::types::AgentEnvironment;
use crate::resolver::semver_utils;
use thiserror::Error;

/// Errors that can occur during manifest validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// The manifest declares no backends.
    #[error("manifest must declare at least one backend")]
    NoBackends,

    /// A plugin references a registry that is not defined in the `registries` section.
    #[error("plugin '{0}' references undefined registry '{1}'")]
    UndefinedRegistry(String, String),

    /// A plugin has neither a registry nor a git source.
    #[error("plugin '{0}' must specify either a registry or git source")]
    NoPluginSource(String),

    /// A plugin has both a registry and a git source.
    #[error("plugin '{0}' specifies both registry and git source — use one or the other")]
    AmbiguousPluginSource(String),

    /// A plugin from a registry is missing its version constraint.
    #[error("plugin '{0}' from registry '{1}' must specify a version constraint")]
    MissingPluginVersion(String, String),

    /// A version constraint string is not valid semver.
    #[error("plugin '{0}' has invalid version constraint '{1}': not a valid semver range")]
    InvalidVersionConstraint(String, String),

    /// A local path reference does not start with `./`.
    #[error("{kind} '{name}' local path must start with './' (got '{path}')")]
    LocalPathMissingPrefix {
        kind: String,
        name: String,
        path: String,
    },

    /// A skill from git is missing a path within the repo.
    #[error("skill '{0}' from git must specify a path within the repo")]
    NoSkillPath(String),

    /// An agent from git is missing a path within the repo.
    #[error("agent '{0}' from git must specify a path within the repo")]
    NoAgentPath(String),

    /// An MCP server has neither a url nor a command.
    #[error("MCP server '{0}' must specify either a url or command")]
    NoMcpSource(String),

    /// An MCP server has both a url and a command.
    #[error("MCP server '{0}' specifies both url and command — use one or the other")]
    AmbiguousMcpSource(String),

    /// The project name is empty.
    #[error("project name must not be empty")]
    EmptyName,

    /// The project name contains invalid characters.
    #[error("project name '{0}' contains invalid characters; use only alphanumeric, hyphens, and underscores")]
    InvalidName(String),

    /// Context section exists but `base` path is empty.
    #[error("context section requires a non-empty base file path")]
    EmptyContextBase,

    /// A context overlay references a backend not declared in `backends`.
    #[error("context overlay references undeclared backend '{0}'")]
    UndeclaredContextBackend(String),

    /// A settings entry references a backend not declared in `backends`.
    #[error("settings entry references undeclared backend '{0}'")]
    UndeclaredSettingsBackend(String),

    /// A registry pin is not a valid hex SHA (must be 40 or 64 hex chars).
    #[error("registry '{0}' has invalid pin '{1}': must be 40 or 64 hex characters")]
    InvalidRegistryPin(String, String),
}

/// Validate a parsed manifest for structural correctness.
///
/// Returns `Ok(())` if the manifest is valid, or a list of all detected
/// validation errors so users can fix them in a single pass.
pub fn validate(manifest: &AgentEnvironment) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    validate_name(manifest, &mut errors);
    validate_backends(manifest, &mut errors);
    validate_registries(manifest, &mut errors);
    validate_plugins(manifest, &mut errors);
    validate_skills(manifest, &mut errors);
    validate_agents(manifest, &mut errors);
    validate_mcp_servers(manifest, &mut errors);
    validate_context(manifest, &mut errors);
    validate_settings(manifest, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Check version constraint syntax for all plugins that declare a version.
///
/// This is a convenience for callers that want version checking separately
/// from full validation.
pub fn check_version_constraints(manifest: &AgentEnvironment) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    for (name, plugin) in &manifest.plugins {
        if let Some(ref version) = plugin.version {
            if semver_utils::parse_constraint(version).is_err() {
                errors.push(ValidationError::InvalidVersionConstraint(
                    name.clone(),
                    version.clone(),
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_name(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    if manifest.name.is_empty() {
        errors.push(ValidationError::EmptyName);
        return;
    }
    let valid = manifest
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !valid {
        errors.push(ValidationError::InvalidName(manifest.name.clone()));
    }
}

fn validate_backends(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    if manifest.backends.is_empty() {
        errors.push(ValidationError::NoBackends);
    }
}

fn validate_registries(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for (name, reg) in &manifest.registries {
        if let Some(ref pin) = reg.pin {
            let valid_length = pin.len() == 40 || pin.len() == 64;
            let all_hex = pin.chars().all(|c| c.is_ascii_hexdigit());
            if !valid_length || !all_hex {
                errors.push(ValidationError::InvalidRegistryPin(
                    name.clone(),
                    pin.clone(),
                ));
            }
        }
    }
}

fn validate_plugins(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for (name, plugin) in &manifest.plugins {
        // Must have exactly one source type
        let has_registry = plugin.registry.is_some();
        let has_git = plugin.git.is_some();

        if !has_registry && !has_git {
            errors.push(ValidationError::NoPluginSource(name.clone()));
            continue;
        }

        if has_registry && has_git {
            errors.push(ValidationError::AmbiguousPluginSource(name.clone()));
        }

        // Registry plugins: check registry exists and version is present + valid
        if let Some(ref registry) = plugin.registry {
            if !manifest.registries.contains_key(registry) {
                errors.push(ValidationError::UndefinedRegistry(
                    name.clone(),
                    registry.clone(),
                ));
            }

            match &plugin.version {
                None => {
                    errors.push(ValidationError::MissingPluginVersion(
                        name.clone(),
                        registry.clone(),
                    ));
                }
                Some(version) => {
                    if semver_utils::parse_constraint(version).is_err() {
                        errors.push(ValidationError::InvalidVersionConstraint(
                            name.clone(),
                            version.clone(),
                        ));
                    }
                }
            }
        }
    }
}

fn validate_skills(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for (name, skill) in &manifest.skills {
        if skill.is_local() {
            // Local paths must start with ./
            if !skill.path.starts_with("./") {
                errors.push(ValidationError::LocalPathMissingPrefix {
                    kind: "skill".to_string(),
                    name: name.clone(),
                    path: skill.path.clone(),
                });
            }
        } else if skill.git.is_none() {
            // Non-local skill without git source — the path alone is ambiguous
            // For git skills, path is the subpath within the repo (no ./ needed)
            // For local skills, path must start with ./
            // If there's no git and no ./, it's unclear what the user meant
            errors.push(ValidationError::LocalPathMissingPrefix {
                kind: "skill".to_string(),
                name: name.clone(),
                path: skill.path.clone(),
            });
        }
    }
}

fn validate_agents(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for (name, agent) in &manifest.agents {
        if agent.is_local() {
            if !agent.path.starts_with("./") {
                errors.push(ValidationError::LocalPathMissingPrefix {
                    kind: "agent".to_string(),
                    name: name.clone(),
                    path: agent.path.clone(),
                });
            }
        } else if agent.git.is_none() {
            errors.push(ValidationError::LocalPathMissingPrefix {
                kind: "agent".to_string(),
                name: name.clone(),
                path: agent.path.clone(),
            });
        }
    }
}

fn validate_mcp_servers(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for (name, server) in &manifest.mcp_servers {
        let has_url = server.url.is_some();
        let has_command = server.command.is_some();

        if !has_url && !has_command {
            errors.push(ValidationError::NoMcpSource(name.clone()));
        }

        if has_url && has_command {
            errors.push(ValidationError::AmbiguousMcpSource(name.clone()));
        }
    }
}

fn validate_context(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    if let Some(ref ctx) = manifest.context {
        if ctx.base.trim().is_empty() {
            errors.push(ValidationError::EmptyContextBase);
        }

        // Check overlay backends are declared
        for backend_name in ctx.overlays.keys() {
            if !manifest.backends.contains_key(backend_name) {
                errors.push(ValidationError::UndeclaredContextBackend(
                    backend_name.clone(),
                ));
            }
        }
    }
}

fn validate_settings(manifest: &AgentEnvironment, errors: &mut Vec<ValidationError>) {
    for backend_name in manifest.settings.keys() {
        if !manifest.backends.contains_key(backend_name) {
            errors.push(ValidationError::UndeclaredSettingsBackend(
                backend_name.clone(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> AgentEnvironment {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn validates_minimal_manifest() {
        let m = parse("name: test\nbackends:\n  claude-code:\n    isolation: strict\n");
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn rejects_no_backends() {
        let m = parse("name: test\n");
        let errs = validate(&m).unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::NoBackends)));
    }

    #[test]
    fn rejects_undefined_registry() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nplugins:\n  foo:\n    registry: nonexistent\n    version: '1.0.0'\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::UndefinedRegistry(_, _))));
    }

    #[test]
    fn rejects_invalid_version_constraint() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  official:\n    url: https://example.com\nplugins:\n  foo:\n    registry: official\n    version: 'not-a-version!!!'\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidVersionConstraint(_, _))));
    }

    #[test]
    fn accepts_valid_version_constraints() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  r:\n    url: https://x.com\nplugins:\n  a:\n    registry: r\n    version: '>=1.0.0, <2.0.0'\n  b:\n    registry: r\n    version: '~1.3.0'\n  c:\n    registry: r\n    version: '^1.0.0'\n  d:\n    registry: r\n    version: '1.0.0'\n",
        );
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn rejects_plugin_missing_version_for_registry() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  r:\n    url: https://x.com\nplugins:\n  foo:\n    registry: r\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::MissingPluginVersion(_, _))));
    }

    #[test]
    fn rejects_ambiguous_plugin_source() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  r:\n    url: https://x.com\nplugins:\n  foo:\n    registry: r\n    version: '1.0.0'\n    git: https://example.com/repo.git\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::AmbiguousPluginSource(_))));
    }

    #[test]
    fn rejects_local_path_without_prefix() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nskills:\n  foo:\n    path: skills/foo\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::LocalPathMissingPrefix { .. })));
    }

    #[test]
    fn accepts_local_skill_with_prefix() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nskills:\n  foo:\n    path: ./skills/foo\n",
        );
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn accepts_git_skill_without_prefix() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nskills:\n  foo:\n    git: https://github.com/org/repo\n    path: skills/foo\n    tag: v1.0.0\n",
        );
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn rejects_mcp_server_with_no_source() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nmcp-servers:\n  broken:\n    args: []\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::NoMcpSource(_))));
    }

    #[test]
    fn rejects_mcp_server_with_both_url_and_command() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nmcp-servers:\n  bad:\n    url: https://example.com\n    command: npx\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::AmbiguousMcpSource(_))));
    }

    #[test]
    fn rejects_empty_context_base() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\ncontext:\n  base: ''\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::EmptyContextBase)));
    }

    #[test]
    fn rejects_context_overlay_for_undeclared_backend() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\ncontext:\n  base: ./CONTEXT.md\n  overlays:\n    nonexistent: ./CONTEXT.nonexistent.md\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::UndeclaredContextBackend(_))));
    }

    #[test]
    fn rejects_settings_for_undeclared_backend() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nsettings:\n  nonexistent:\n    foo: bar\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::UndeclaredSettingsBackend(_))));
    }

    #[test]
    fn rejects_empty_name() {
        let m = parse("name: ''\nbackends:\n  claude-code: {}\n");
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::EmptyName)));
    }

    #[test]
    fn rejects_invalid_name_characters() {
        let m = parse("name: 'my project!'\nbackends:\n  claude-code: {}\n");
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidName(_))));
    }

    #[test]
    fn accepts_valid_name_with_hyphens_and_underscores() {
        let m = parse("name: my-cool_project123\nbackends:\n  claude-code: {}\n");
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn accepts_valid_registry_pin_40_chars() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  official:\n    url: https://example.com\n    pin: abcdef1234567890abcdef1234567890abcdef12\n",
        );
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn accepts_valid_registry_pin_64_chars() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  official:\n    url: https://example.com\n    pin: abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890\n",
        );
        assert!(validate(&m).is_ok());
    }

    #[test]
    fn rejects_registry_pin_wrong_length() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  official:\n    url: https://example.com\n    pin: abc123\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidRegistryPin(_, _))));
    }

    #[test]
    fn rejects_registry_pin_non_hex() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  official:\n    url: https://example.com\n    pin: xyz1231234567890abcdef1234567890abcdef12\n",
        );
        let errs = validate(&m).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidRegistryPin(_, _))));
    }

    #[test]
    fn check_version_constraints_valid() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nregistries:\n  r:\n    url: https://x.com\nplugins:\n  a:\n    registry: r\n    version: '^1.0.0'\n",
        );
        assert!(check_version_constraints(&m).is_ok());
    }

    #[test]
    fn check_version_constraints_invalid() {
        let m = parse(
            "name: test\nbackends:\n  claude-code: {}\nplugins:\n  a:\n    registry: r\n    version: 'garbage!!'\n",
        );
        let errs = check_version_constraints(&m).unwrap_err();
        assert_eq!(errs.len(), 1);
    }
}
