//! Integration tests for manifest loading, validation, and round-trip serialization.

use std::path::Path;

// Import the agx crate's public modules.
// Note: for integration tests to access these, the modules must be pub in main.rs
// or we test via the binary. Since this is a binary crate, we re-parse YAML directly
// using the same types.

use agx::manifest;
use agx::manifest::types::AgentEnvironment;
use agx::manifest::validate::{self, ValidationError};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

// --- Loading fixtures ---

#[test]
fn load_minimal_fixture() {
    let env = manifest::load(&fixture("minimal.yml")).unwrap();
    assert_eq!(env.name, "minimal-project");
    assert_eq!(env.backends.len(), 1);
    assert!(env.backends.contains_key("claude-code"));
    assert!(env.plugins.is_empty());
    assert!(env.skills.is_empty());
    assert!(env.agents.is_empty());
    assert!(env.mcp_servers.is_empty());
}

#[test]
fn load_full_fixture() {
    let env = manifest::load(&fixture("full.yml")).unwrap();
    assert_eq!(env.name, "my-agent-project");
    assert_eq!(env.version.as_deref(), Some("0.1.0"));
    assert_eq!(
        env.description.as_deref(),
        Some("An agent environment for the Acme backend service")
    );
    assert_eq!(env.backends.len(), 2);
    assert_eq!(env.registries.len(), 3);
    assert_eq!(env.plugins.len(), 4);
    assert_eq!(env.skills.len(), 3);
    assert_eq!(env.agents.len(), 3);
    assert_eq!(env.mcp_servers.len(), 3);
    assert!(env.context.is_some());
    assert!(env.paths.is_some());

    // Check specific plugin sources
    let qr = &env.plugins["quality-review"];
    assert_eq!(qr.registry.as_deref(), Some("official"));
    assert_eq!(qr.version.as_deref(), Some(">=1.0.0, <2.0.0"));

    let cp = &env.plugins["custom-plugin"];
    assert!(cp.git.is_some());
    assert_eq!(cp.tag.as_deref(), Some("v3.0.0"));

    // Check skill sources
    assert!(env.skills["deploy-checklist"].is_local());
    assert!(!env.skills["web-design-guidelines"].is_local());

    // Check agent model overrides
    assert_eq!(env.agents["my-reviewer"].model.as_deref(), Some("opus"));
    assert_eq!(env.agents["quick-linter"].model.as_deref(), Some("haiku"));

    // Check MCP servers
    assert!(env.mcp_servers["playwright"].url.is_some());
    assert!(env.mcp_servers["filesystem"].command.is_some());
}

#[test]
fn load_invalid_no_backends_fixture() {
    let env = manifest::load(&fixture("invalid-no-backends.yml")).unwrap();
    // File parses fine — backends default to empty BTreeMap
    assert!(env.backends.is_empty());
}

#[test]
fn load_invalid_undefined_registry_fixture() {
    let env = manifest::load(&fixture("invalid-undefined-registry.yml")).unwrap();
    assert!(env.registries.is_empty());
    assert_eq!(env.plugins.len(), 1);
}

// --- Validation ---

#[test]
fn validate_minimal_fixture_passes() {
    let env = manifest::load(&fixture("minimal.yml")).unwrap();
    assert!(validate::validate(&env).is_ok());
}

#[test]
fn validate_full_fixture_passes() {
    let env = manifest::load(&fixture("full.yml")).unwrap();
    assert!(validate::validate(&env).is_ok());
}

#[test]
fn validate_no_backends_fixture_fails() {
    let env = manifest::load(&fixture("invalid-no-backends.yml")).unwrap();
    let errs = validate::validate(&env).unwrap_err();
    assert!(errs.iter().any(|e| matches!(e, ValidationError::NoBackends)));
}

#[test]
fn validate_undefined_registry_fixture_fails() {
    let env = manifest::load(&fixture("invalid-undefined-registry.yml")).unwrap();
    let errs = validate::validate(&env).unwrap_err();
    assert!(errs.iter().any(|e| matches!(e, ValidationError::UndefinedRegistry(_, _))));
}

// --- Round-trip: load -> serialize -> load -> assert equality ---

#[test]
fn round_trip_minimal() {
    let original = manifest::load(&fixture("minimal.yml")).unwrap();
    let yaml = serde_yaml::to_string(&original).unwrap();
    let reloaded: AgentEnvironment = serde_yaml::from_str(&yaml).unwrap();

    assert_eq!(original.name, reloaded.name);
    assert_eq!(original.backends.len(), reloaded.backends.len());
    for key in original.backends.keys() {
        assert!(reloaded.backends.contains_key(key));
    }
}

#[test]
fn round_trip_full() {
    let original = manifest::load(&fixture("full.yml")).unwrap();
    let yaml = serde_yaml::to_string(&original).unwrap();
    let reloaded: AgentEnvironment = serde_yaml::from_str(&yaml).unwrap();

    // Core metadata
    assert_eq!(original.name, reloaded.name);
    assert_eq!(original.version, reloaded.version);
    assert_eq!(original.description, reloaded.description);

    // Section counts
    assert_eq!(original.backends.len(), reloaded.backends.len());
    assert_eq!(original.registries.len(), reloaded.registries.len());
    assert_eq!(original.plugins.len(), reloaded.plugins.len());
    assert_eq!(original.skills.len(), reloaded.skills.len());
    assert_eq!(original.agents.len(), reloaded.agents.len());
    assert_eq!(original.mcp_servers.len(), reloaded.mcp_servers.len());

    // All keys preserved
    for key in original.plugins.keys() {
        assert!(reloaded.plugins.contains_key(key), "missing plugin: {key}");
    }
    for key in original.skills.keys() {
        assert!(reloaded.skills.contains_key(key), "missing skill: {key}");
    }
    for key in original.agents.keys() {
        assert!(reloaded.agents.contains_key(key), "missing agent: {key}");
    }
    for key in original.mcp_servers.keys() {
        assert!(
            reloaded.mcp_servers.contains_key(key),
            "missing mcp server: {key}"
        );
    }

    // Spot-check specific values survive round-trip
    assert_eq!(
        original.plugins["quality-review"].version,
        reloaded.plugins["quality-review"].version,
    );
    assert_eq!(
        original.agents["my-reviewer"].model,
        reloaded.agents["my-reviewer"].model,
    );
    assert_eq!(
        original.skills["deploy-checklist"].path,
        reloaded.skills["deploy-checklist"].path,
    );

    // Context round-trip
    let orig_ctx = original.context.as_ref().unwrap();
    let reload_ctx = reloaded.context.as_ref().unwrap();
    assert_eq!(orig_ctx.base, reload_ctx.base);
    assert_eq!(orig_ctx.overlays.len(), reload_ctx.overlays.len());
    assert_eq!(orig_ctx.children.len(), reload_ctx.children.len());
}
