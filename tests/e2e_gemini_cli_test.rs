//! End-to-end tests that verify Gemini CLI correctly interprets agx-generated files.
//!
//! These tests require:
//!   - Gemini CLI installed (the `gemini` binary)
//!   - A valid API key configured
//!   - The `AGX_E2E_TESTS` environment variable set to `1`
//!
//! Run with:
//!   AGX_E2E_TESTS=1 cargo test --test e2e_gemini_cli_test
//!
//! These tests shell out to `gemini -p` to verify that generated settings and
//! context files are loaded correctly by Gemini CLI at runtime.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use agx::backends::{get_backend, ResolvedEnvironment};
use agx::context::generate_context_files;
use agx::manifest;
use agx::resolver::{resolve_all, ResolveContext};

/// Skip the test if AGX_E2E_TESTS is not set.
fn should_skip() -> bool {
    std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1")
}

/// Find the `gemini` binary on the system.
fn gemini_bin() -> String {
    for path in &["/opt/homebrew/bin/gemini", "/usr/local/bin/gemini"] {
        if Path::new(path).exists() {
            return path.to_string();
        }
    }
    "gemini".to_string()
}

/// Run a prompt through Gemini CLI in the given directory and return stdout.
fn ask_gemini(dir: &Path, prompt: &str) -> String {
    let output = Command::new(gemini_bin())
        .args(["-p", prompt])
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run gemini");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Set up a project directory with agx-generated Gemini CLI files.
fn setup_project(dir: &Path, manifest_yaml: &str) {
    fs::write(dir.join("agent-environment.yml"), manifest_yaml).unwrap();

    let manifest = manifest::load(&dir.join("agent-environment.yml")).unwrap();

    // Create local skill/agent source files if referenced
    for (name, skill) in &manifest.skills {
        let skill_dir = dir.join(&skill.path);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: The {name} skill for testing.\n---\n# {name}\nThis is the {name} skill.\n"
            ),
        )
        .unwrap();
    }
    for (_name, agent) in &manifest.agents {
        let agent_path = dir.join(&agent.path);
        if let Some(parent) = agent_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&agent_path, "Review the code carefully.\n").unwrap();
    }

    // Create context file if referenced
    if let Some(ref ctx) = manifest.context {
        fs::write(
            dir.join(&ctx.base),
            "# Gemini E2E Test Project\nThis project tests agx with Gemini CLI backend.\n",
        )
        .unwrap();
        for (_backend, overlay_path) in &ctx.overlays {
            fs::write(
                dir.join(overlay_path),
                "# Backend-specific instructions\nFollow these backend guidelines.\n",
            )
            .unwrap();
        }
    }

    // Resolve and generate
    let context = ResolveContext {
        registry_versions: BTreeMap::new(),
        git_commits: BTreeMap::new(),
    };
    let plugin_assets = BTreeMap::new();
    let lockfile = resolve_all(&manifest, &context, &plugin_assets).unwrap();

    let cache_dir = tempdir().unwrap();
    let resolved = ResolvedEnvironment {
        manifest: &manifest,
        lockfile: &lockfile,
        cache_dir: cache_dir.path(),
    };

    let backend = get_backend("gemini-cli").unwrap();
    backend.generate(&resolved, dir).unwrap();

    generate_context_files(&manifest, dir).unwrap();
}

// ---------------------------------------------------------------------------
// E2E Test 1: Gemini loads GEMINI.md context
// ---------------------------------------------------------------------------

#[test]
fn e2e_gemini_loads_context() {
    if should_skip() {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: gemini-context-test
backends:
  gemini-cli: {}
context:
  base: ./CONTEXT.md
"#,
    );

    // Verify GEMINI.md was generated
    let gemini_md = fs::read_to_string(tmp.path().join("GEMINI.md")).unwrap();
    assert!(gemini_md.contains("Gemini E2E Test Project"));

    // Ask Gemini what project it's working in
    let response = ask_gemini(
        tmp.path(),
        "Based on your GEMINI.md context, what kind of project is this? Reply in one sentence.",
    );

    assert!(
        response.to_lowercase().contains("test")
            || response.to_lowercase().contains("agx")
            || response.to_lowercase().contains("gemini"),
        "Gemini should recognize the project context. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 2: Settings.json is valid and loaded without errors
// ---------------------------------------------------------------------------

#[test]
fn e2e_gemini_settings_valid() {
    if should_skip() {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: gemini-settings-test
backends:
  gemini-cli:
    model: gemini-2.5-flash
context:
  base: ./CONTEXT.md
"#,
    );

    // Verify settings.json uses the correct model object format
    let settings = fs::read_to_string(tmp.path().join(".gemini/settings.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(parsed["model"]["name"], "gemini-2.5-flash");

    // Gemini should start without settings validation errors
    let response = ask_gemini(tmp.path(), "Say hello in one word.");

    // If settings were invalid, gemini would error out and return empty or error text
    assert!(
        !response.is_empty() && !response.contains("Invalid configuration"),
        "Gemini should accept the settings.json without errors. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 3: Model with full config and parameters
// ---------------------------------------------------------------------------

#[test]
fn e2e_gemini_full_model_config() {
    if should_skip() {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: gemini-model-test
backends:
  gemini-cli:
    model:
      id: gemini-2.5-flash
      parameters:
        temperature: 0.5
context:
  base: ./CONTEXT.md
"#,
    );

    // Verify settings.json has modelConfigs
    let settings = fs::read_to_string(tmp.path().join(".gemini/settings.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(parsed["model"]["name"], "gemini-2.5-flash");
    assert_eq!(parsed["modelConfigs"]["gemini-2.5-flash"]["temperature"], 0.5);

    // Should work without errors
    let response = ask_gemini(tmp.path(), "Say hello in one word.");
    assert!(
        !response.is_empty() && !response.contains("Invalid configuration"),
        "Gemini should accept full model config. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 4: Environment isolation between projects
// ---------------------------------------------------------------------------

#[test]
fn e2e_gemini_environment_isolation() {
    if should_skip() {
        return;
    }

    let tmp_a = tempdir().unwrap();
    let tmp_b = tempdir().unwrap();

    setup_project(
        tmp_a.path(),
        r#"
name: gemini-project-alpha
backends:
  gemini-cli: {}
context:
  base: ./CONTEXT.md
"#,
    );

    // Write distinct context for project A
    fs::write(
        tmp_a.path().join("CONTEXT.md"),
        "# Alpha Project\nThis project manages rocket launches.\n",
    )
    .unwrap();
    let manifest_a = manifest::load(&tmp_a.path().join("agent-environment.yml")).unwrap();
    generate_context_files(&manifest_a, tmp_a.path()).unwrap();

    setup_project(
        tmp_b.path(),
        r#"
name: gemini-project-beta
backends:
  gemini-cli: {}
context:
  base: ./CONTEXT.md
"#,
    );

    // Write distinct context for project B
    fs::write(
        tmp_b.path().join("CONTEXT.md"),
        "# Beta Project\nThis project manages submarine navigation.\n",
    )
    .unwrap();
    let manifest_b = manifest::load(&tmp_b.path().join("agent-environment.yml")).unwrap();
    generate_context_files(&manifest_b, tmp_b.path()).unwrap();

    // Project A should know about rockets
    let response_a = ask_gemini(
        tmp_a.path(),
        "Based on your GEMINI.md, what does this project manage? Reply in 3 words or fewer.",
    );
    assert!(
        response_a.to_lowercase().contains("rocket"),
        "Project A should know about rockets. Got: {response_a}"
    );

    // Project B should know about submarines
    let response_b = ask_gemini(
        tmp_b.path(),
        "Based on your GEMINI.md, what does this project manage? Reply in 3 words or fewer.",
    );
    assert!(
        response_b.to_lowercase().contains("submarine"),
        "Project B should know about submarines. Got: {response_b}"
    );
}
