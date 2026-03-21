//! Integration tests for git source operations and registry index.
//!
//! These tests create temporary git repos to exercise clone, fetch,
//! resolve_ref, resolve_default_branch, extract, and registry functions.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use agx::sources::git;
use agx::sources::registry;

/// Helper: initialize a non-bare git repo in a temp dir with an initial commit.
/// Returns the temp dir (repo root).
fn create_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "test@test.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    // Create an initial commit with a file
    std::fs::write(repo.join("hello.txt"), "hello world\n").unwrap();
    run_git(repo, &["add", "hello.txt"]);
    run_git(repo, &["commit", "-m", "initial commit"]);

    dir
}

/// Helper: run a git command in a directory, panicking on failure.
fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));
    assert!(
        status.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&status.stderr)
    );
}

/// Helper: get the HEAD SHA of a repo.
fn head_sha(dir: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// --- Clone tests ---

#[test]
fn test_clone_bare() {
    let source = create_test_repo();
    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");

    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    // Bare repo should exist and have HEAD
    assert!(bare_path.join("HEAD").exists());
}

// --- Fetch tests ---

#[test]
fn test_fetch_updates() {
    let source = create_test_repo();
    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");

    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let sha_before = git::resolve_default_branch(&bare_path).unwrap();

    // Add a new commit to the source repo
    std::fs::write(source.path().join("new.txt"), "new file\n").unwrap();
    run_git(source.path(), &["add", "new.txt"]);
    run_git(source.path(), &["commit", "-m", "second commit"]);

    let source_sha_after = head_sha(source.path());
    assert_ne!(sha_before, source_sha_after);

    // Fetch should pick up the new commit
    git::fetch(&bare_path).unwrap();

    // After fetch in a bare repo, the local branch heads are updated
    // (since git fetch --all updates remote-tracking refs).
    // Resolve HEAD which should now point to the updated default branch.
    let resolved = git::resolve_default_branch(&bare_path).unwrap();
    assert_eq!(resolved, source_sha_after);
}

// --- Resolve ref tests ---

#[test]
fn test_resolve_ref_tag() {
    let source = create_test_repo();
    let expected_sha = head_sha(source.path());

    run_git(source.path(), &["tag", "v1.0.0"]);

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let resolved = git::resolve_ref(&bare_path, "v1.0.0").unwrap();
    assert_eq!(resolved, expected_sha);
}

#[test]
fn test_resolve_ref_commit_prefix() {
    let source = create_test_repo();
    let full_sha = head_sha(source.path());
    let prefix = &full_sha[..8];

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let resolved = git::resolve_ref(&bare_path, prefix).unwrap();
    assert_eq!(resolved, full_sha);
}

#[test]
fn test_resolve_ref_nonexistent() {
    let source = create_test_repo();
    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let result = git::resolve_ref(&bare_path, "nonexistent-ref");
    assert!(result.is_err());
}

// --- Resolve default branch tests ---

#[test]
fn test_resolve_default_branch() {
    let source = create_test_repo();
    let expected_sha = head_sha(source.path());

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let resolved = git::resolve_default_branch(&bare_path).unwrap();
    assert_eq!(resolved, expected_sha);
}

// --- Extract tests ---

#[test]
fn test_extract_file() {
    let source = create_test_repo();
    let sha = head_sha(source.path());

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let extract_dir = TempDir::new().unwrap();
    git::extract(&bare_path, &sha, "hello.txt", extract_dir.path()).unwrap();

    let content = std::fs::read_to_string(extract_dir.path().join("hello.txt")).unwrap();
    assert_eq!(content, "hello world\n");
}

#[test]
fn test_extract_subdirectory() {
    let source = create_test_repo();

    // Add a subdirectory with files
    let subdir = source.path().join("plugins").join("my-plugin");
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(subdir.join("plugin.yml"), "name: my-plugin\n").unwrap();
    std::fs::write(subdir.join("CONTEXT.md"), "# Plugin context\n").unwrap();
    run_git(source.path(), &["add", "."]);
    run_git(source.path(), &["commit", "-m", "add plugin"]);

    let sha = head_sha(source.path());

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let extract_dir = TempDir::new().unwrap();
    git::extract(&bare_path, &sha, "plugins/my-plugin", extract_dir.path()).unwrap();

    assert!(extract_dir
        .path()
        .join("plugins/my-plugin/plugin.yml")
        .exists());
    assert!(extract_dir
        .path()
        .join("plugins/my-plugin/CONTEXT.md")
        .exists());
}

#[test]
fn test_extract_nonexistent_path() {
    let source = create_test_repo();
    let sha = head_sha(source.path());

    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    let extract_dir = TempDir::new().unwrap();
    let result = git::extract(&bare_path, &sha, "nonexistent", extract_dir.path());
    assert!(result.is_err());
}

// --- Full clone → resolve → extract pipeline ---

#[test]
fn test_clone_resolve_extract_pipeline() {
    let source = create_test_repo();

    // Add tagged content
    std::fs::write(source.path().join("data.txt"), "v1 data\n").unwrap();
    run_git(source.path(), &["add", "data.txt"]);
    run_git(source.path(), &["commit", "-m", "add data"]);
    run_git(source.path(), &["tag", "v1.0.0"]);

    // Add more content after the tag
    std::fs::write(source.path().join("data.txt"), "v2 data\n").unwrap();
    run_git(source.path(), &["add", "data.txt"]);
    run_git(source.path(), &["commit", "-m", "update data"]);

    // Clone
    let dest_dir = TempDir::new().unwrap();
    let bare_path = dest_dir.path().join("repo.git");
    git::clone(source.path().to_str().unwrap(), &bare_path).unwrap();

    // Resolve the tag
    let sha = git::resolve_ref(&bare_path, "v1.0.0").unwrap();

    // Extract at the tagged commit — should get v1 data, not v2
    let extract_dir = TempDir::new().unwrap();
    git::extract(&bare_path, &sha, "data.txt", extract_dir.path()).unwrap();

    let content = std::fs::read_to_string(extract_dir.path().join("data.txt")).unwrap();
    assert_eq!(content, "v1 data\n");
}

// --- Registry integration tests ---

#[test]
fn test_registry_fetch_and_lookup() {
    // Create a source repo that acts as a registry
    let source = create_test_repo();

    let marketplace = r#"{
        "plugins": {
            "quality-review": {
                "description": "Code quality review",
                "versions": {
                    "1.0.0": { "commit": "abc123", "path": "plugins/quality-review" },
                    "0.9.0": { "commit": "def456", "path": "plugins/quality-review" }
                }
            }
        }
    }"#;

    std::fs::create_dir_all(source.path().join(".claude-plugin")).unwrap();
    std::fs::write(source.path().join(".claude-plugin/marketplace.json"), marketplace).unwrap();
    run_git(source.path(), &["add", ".claude-plugin/marketplace.json"]);
    run_git(source.path(), &["commit", "-m", "add marketplace"]);

    // Fetch the index
    let cache_dir = TempDir::new().unwrap();
    let registry_path = cache_dir.path().join("test-registry.git");
    let index =
        registry::fetch_index(source.path().to_str().unwrap(), &registry_path).unwrap();

    // Lookup
    let versions = registry::lookup_plugin(&index, "quality-review").unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, semver::Version::new(1, 0, 0));
    assert_eq!(versions[0].git_commit, "abc123");
    assert_eq!(versions[1].version, semver::Version::new(0, 9, 0));

    // Not found
    assert!(registry::lookup_plugin(&index, "nonexistent").is_none());
}

// --- URL slug tests (also tested as unit tests in git.rs) ---

#[test]
fn test_url_to_slug() {
    assert_eq!(
        git::url_to_slug("https://github.com/org/repo"),
        "github.com-org-repo.git"
    );
    assert_eq!(
        git::url_to_slug("git@github.com:org/repo.git"),
        "github.com-org-repo.git"
    );
}
