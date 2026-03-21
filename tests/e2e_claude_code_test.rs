//! End-to-end tests that verify Claude Code correctly interprets agx-generated files.
//!
//! These tests require:
//!   - Claude Code installed (the `claude` binary)
//!   - A valid API key configured
//!   - The `AGX_E2E_TESTS` environment variable set to `1`
//!
//! Run with:
//!   AGX_E2E_TESTS=1 cargo test --test e2e_claude_code_test
//!
//! These tests shell out to `claude -p --dangerously-skip-permissions` to verify
//! that generated settings, context, skills, and environment variables are loaded
//! correctly by Claude Code at runtime.

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
fn require_e2e() {
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping e2e test (set AGX_E2E_TESTS=1 to enable)");
        return;
    }
}

/// Find the `claude` binary on the system.
fn claude_bin() -> String {
    // Check common locations, fall back to PATH
    for path in &[
        "/Users/bdc/.local/bin/claude",
        "/usr/local/bin/claude",
    ] {
        if Path::new(path).exists() {
            return path.to_string();
        }
    }
    // Fall back to PATH lookup
    "claude".to_string()
}

/// Run a prompt through Claude Code in the given directory and return stdout.
fn ask_claude(dir: &Path, prompt: &str) -> String {
    let output = Command::new(claude_bin())
        .args(["-p", "--dangerously-skip-permissions"])
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(prompt.as_bytes()).ok();
            }
            child.wait_with_output()
        })
        .expect("failed to run claude");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Set up a project directory with agx-generated files.
fn setup_project(dir: &Path, manifest_yaml: &str) {
    fs::write(dir.join("agent-environment.yml"), manifest_yaml).unwrap();

    let manifest = manifest::load(&dir.join("agent-environment.yml")).unwrap();

    // Create local skill/agent source files if referenced
    for (name, skill) in &manifest.skills {
        let skill_dir = dir.join(&skill.path);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: The {name} skill for testing.\n---\n# {name}\nThis is the {name} skill.\n"),
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
            "# E2E Test Project\nThis project tests agx environment generation.\n",
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

    let backend = get_backend("claude-code").unwrap();
    backend.generate(&resolved, dir).unwrap();

    generate_context_files(&manifest, dir).unwrap();
}

// ---------------------------------------------------------------------------
// E2E Test 1: Claude loads CLAUDE.md context
// ---------------------------------------------------------------------------

#[test]
fn e2e_claude_loads_context() {
    require_e2e();
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: context-test-project
backends:
  claude-code:
    isolation: strict
context:
  base: ./CONTEXT.md
"#,
    );

    // Verify CLAUDE.md was generated
    let claude_md = fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert!(claude_md.contains("E2E Test Project"));

    // Ask Claude what project it's working in
    let response = ask_claude(
        tmp.path(),
        "Based on your CLAUDE.md context, what kind of project is this? Reply in one sentence.",
    );

    assert!(
        response.to_lowercase().contains("test")
            || response.to_lowercase().contains("agx")
            || response.to_lowercase().contains("environment"),
        "Claude should recognize the project context. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 2: Claude sees AGX_PROJECT environment variable
// ---------------------------------------------------------------------------

#[test]
fn e2e_claude_sees_agx_project_env() {
    require_e2e();
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: env-var-test
backends:
  claude-code:
    isolation: strict
"#,
    );

    // Verify settings.json has the env var
    let settings = fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(settings.contains("AGX_PROJECT"));
    assert!(settings.contains("env-var-test"));

    // Ask Claude to read the env var
    let response = ask_claude(
        tmp.path(),
        "Run: echo $AGX_PROJECT — reply with ONLY the output value, nothing else.",
    );

    assert!(
        response.contains("env-var-test"),
        "AGX_PROJECT should be 'env-var-test'. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 3: Claude sees agx-generated skills as slash commands
// ---------------------------------------------------------------------------

#[test]
fn e2e_claude_sees_skills() {
    require_e2e();
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: skills-test
backends:
  claude-code:
    isolation: strict
skills:
  my-custom-skill:
    path: ./skills/my-custom-skill
"#,
    );

    // Verify skill was generated
    assert!(tmp
        .path()
        .join(".claude/skills/my-custom-skill/SKILL.md")
        .exists());

    // Ask Claude to list its slash commands
    let response = ask_claude(
        tmp.path(),
        "List all your available slash commands. Include every one. Reply with just the command names, one per line.",
    );

    assert!(
        response.contains("my-custom-skill"),
        "Claude should see /my-custom-skill as a slash command. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 4: Strict isolation disables background tasks
// ---------------------------------------------------------------------------

#[test]
fn e2e_strict_isolation_env_vars() {
    require_e2e();
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        return;
    }

    let tmp = tempdir().unwrap();
    setup_project(
        tmp.path(),
        r#"
name: strict-iso-test
backends:
  claude-code:
    isolation: strict
"#,
    );

    // Verify settings.json has strict isolation flags
    let settings = fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(settings.contains("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS"));

    // Verify Claude sees the env var
    let response = ask_claude(
        tmp.path(),
        "Run: echo $CLAUDE_CODE_DISABLE_BACKGROUND_TASKS — reply with ONLY the output value.",
    );

    assert!(
        response.contains("1"),
        "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS should be '1'. Got: {response}"
    );
}

// ---------------------------------------------------------------------------
// E2E Test 5: Environment isolation between projects
// ---------------------------------------------------------------------------

#[test]
fn e2e_environment_isolation() {
    require_e2e();
    if std::env::var("AGX_E2E_TESTS").as_deref() != Ok("1") {
        return;
    }

    let tmp_a = tempdir().unwrap();
    let tmp_b = tempdir().unwrap();

    setup_project(
        tmp_a.path(),
        r#"
name: project-alpha
backends:
  claude-code:
    isolation: strict
skills:
  alpha-skill:
    path: ./skills/alpha-skill
context:
  base: ./CONTEXT.md
"#,
    );

    // Write distinct context for project A
    fs::write(
        tmp_a.path().join("CONTEXT.md"),
        "# Alpha Project\nThis project manages alpha workflows.\n",
    )
    .unwrap();
    // Regenerate context
    let manifest_a = manifest::load(&tmp_a.path().join("agent-environment.yml")).unwrap();
    generate_context_files(&manifest_a, tmp_a.path()).unwrap();

    setup_project(
        tmp_b.path(),
        r#"
name: project-beta
backends:
  claude-code:
    isolation: strict
skills:
  beta-skill:
    path: ./skills/beta-skill
context:
  base: ./CONTEXT.md
"#,
    );

    // Write distinct context for project B
    fs::write(
        tmp_b.path().join("CONTEXT.md"),
        "# Beta Project\nThis project manages beta workflows.\n",
    )
    .unwrap();
    let manifest_b = manifest::load(&tmp_b.path().join("agent-environment.yml")).unwrap();
    generate_context_files(&manifest_b, tmp_b.path()).unwrap();

    // Project A should see alpha-skill but not beta-skill
    let response_a = ask_claude(
        tmp_a.path(),
        "List all your available slash commands. Reply with just the command names, one per line.",
    );
    assert!(
        response_a.contains("alpha-skill"),
        "Project A should see /alpha-skill. Got: {response_a}"
    );
    assert!(
        !response_a.contains("beta-skill"),
        "Project A should NOT see /beta-skill. Got: {response_a}"
    );

    // Project B should see beta-skill but not alpha-skill
    let response_b = ask_claude(
        tmp_b.path(),
        "List all your available slash commands. Reply with just the command names, one per line.",
    );
    assert!(
        response_b.contains("beta-skill"),
        "Project B should see /beta-skill. Got: {response_b}"
    );
    assert!(
        !response_b.contains("alpha-skill"),
        "Project B should NOT see /alpha-skill. Got: {response_b}"
    );

    // AGX_PROJECT should differ between projects
    let env_a = ask_claude(
        tmp_a.path(),
        "Run: echo $AGX_PROJECT — reply with ONLY the output value.",
    );
    let env_b = ask_claude(
        tmp_b.path(),
        "Run: echo $AGX_PROJECT — reply with ONLY the output value.",
    );

    assert!(
        env_a.contains("project-alpha"),
        "Project A AGX_PROJECT should be 'project-alpha'. Got: {env_a}"
    );
    assert!(
        env_b.contains("project-beta"),
        "Project B AGX_PROJECT should be 'project-beta'. Got: {env_b}"
    );
}
