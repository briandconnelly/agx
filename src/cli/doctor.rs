//! `agx doctor` — diagnose system, user, and project configuration.
//!
//! Checks backend binary availability, user-level configs that may influence
//! (or be blocked by) the project's isolation mode, and project health
//! (manifest validity, lockfile staleness, cache integrity, git availability).
//!
//! With `--fix`, attempts to automatically resolve detected issues.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args;
use console::style;

use agx::cache;
use agx::config;
use agx::manifest;
use agx::manifest::types::{AgentEnvironment, IsolationMode};

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Attempt to automatically fix detected issues.
    #[arg(long)]
    pub fix: bool,
}

/// Kinds of issues that `--fix` can resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fix {
    /// Run `agx init -y` to create a manifest.
    CreateManifest,
    /// Run `agx install` to create or update the lockfile.
    RunInstall,
    /// Run `agx generate` to regenerate backend output.
    RunGenerate,
    /// Install a backend binary via npm.
    InstallBinary { backend: String },
    /// Add `.agx/` to `.gitignore`.
    AddAgxToGitignore,
    /// Stage `agent-environment.lock` in git.
    TrackLockfile,
}

/// Tracks pass / warn / fail counts and queued fixes.
#[derive(Default)]
struct Report {
    pass: usize,
    warn: usize,
    fail: usize,
    fixes: Vec<Fix>,
}

impl Report {
    fn pass(&mut self, msg: &str) {
        self.pass += 1;
        println!("  {} {}", style("\u{2713}").green().bold(), msg);
    }

    fn warn(&mut self, msg: &str) {
        self.warn += 1;
        println!("  {} {}", style("\u{26a0}").yellow().bold(), msg);
    }

    fn warn_fixable(&mut self, msg: &str, fix: Fix) {
        self.warn += 1;
        println!("  {} {}", style("\u{26a0}").yellow().bold(), msg);
        self.fixes.push(fix);
    }

    fn fail(&mut self, msg: &str) {
        self.fail += 1;
        println!("  {} {}", style("\u{2717}").red().bold(), msg);
    }

    fn fail_fixable(&mut self, msg: &str, fix: Fix) {
        self.fail += 1;
        println!("  {} {}", style("\u{2717}").red().bold(), msg);
        self.fixes.push(fix);
    }

    fn detail(&self, msg: &str) {
        println!("    {}", style(msg).dim());
    }

    fn print_summary(&self) {
        println!();
        if self.fail > 0 {
            println!(
                "{} {} problem{}, {} warning{}",
                style("\u{2717}").red().bold(),
                self.fail,
                if self.fail == 1 { "" } else { "s" },
                self.warn,
                if self.warn == 1 { "" } else { "s" },
            );
        } else if self.warn > 0 {
            println!(
                "{} No problems found, {} warning{}",
                style("\u{2713}").green().bold(),
                self.warn,
                if self.warn == 1 { "" } else { "s" },
            );
        } else {
            println!(
                "{} No problems found.",
                style("\u{2713}").green().bold(),
            );
        }
    }
}

/// Run the `agx doctor` command.
pub fn run(args: DoctorArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args.fix)
}

/// Doctor implementation, separated for testability.
pub fn run_in(start_dir: &Path, fix: bool) -> anyhow::Result<()> {
    let mut report = Report::default();

    // --- System checks ---
    println!("{}", style("System").bold());
    check_git(&mut report);

    // --- Project checks ---
    let manifest_path = config::find_manifest(start_dir);
    let project = manifest_path.as_ref().and_then(|path| {
        let root = path.parent().unwrap().to_path_buf();
        match manifest::load(path) {
            Ok(env) => Some((env, path.clone(), root)),
            Err(e) => {
                report.fail(&format!("Manifest failed to parse: {e}"));
                None
            }
        }
    });

    if let Some((ref env, ref manifest_path, ref project_root)) = project {
        // Check binaries for all declared backends.
        check_backend_binaries(env, &mut report);

        println!();
        println!("{}", style(format!("Project: {}", env.name)).bold());
        check_manifest_validity(env, &mut report);
        check_lockfile(manifest_path, project_root, env, &mut report);
        check_cache_integrity(project_root, &mut report);
        check_generated_files(project_root, env, &mut report);
        check_gitignore(project_root, &mut report);
        check_lockfile_tracked(project_root, &mut report);

        // Per-backend user config checks.
        for (name, cfg) in &env.backends {
            let isolation = cfg.isolation.clone().unwrap_or_default();
            println!();
            println!(
                "{}",
                style(format!("Backend: {name} (isolation: {isolation})")).bold()
            );
            check_user_config(name, &isolation, env, &mut report);
        }
    } else if manifest_path.is_none() {
        println!();
        println!("{}", style("Project").bold());
        report.warn_fixable(
            &format!(
                "No {} found \u{2014} run {} to create one",
                config::MANIFEST_FILE,
                style("agx init").cyan(),
            ),
            Fix::CreateManifest,
        );

        // Still check common backend binaries.
        let mut dummy = AgentEnvironment {
            name: String::new(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
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
        };
        for name in &["claude-code", "gemini-cli", "codex"] {
            dummy.backends.insert(
                name.to_string(),
                agx::manifest::types::BackendConfig {
                    default: None,
                    isolation: None,
                    model: None,
                },
            );
        }
        check_backend_binaries(&dummy, &mut report);
    }

    report.print_summary();

    // --- Apply fixes ---
    if fix && !report.fixes.is_empty() {
        // Deduplicate: if we need both install and generate, install covers generate.
        let mut fixes = report.fixes.clone();
        if fixes.contains(&Fix::RunInstall) {
            fixes.retain(|f| !matches!(f, Fix::RunGenerate));
        }

        println!();
        println!("{}", style("Fixing...").bold());
        for f in &fixes {
            apply_fix(f, start_dir)?;
        }
    } else if !fix && !report.fixes.is_empty() {
        println!(
            "\nRun {} to attempt automatic fixes.",
            style("agx doctor --fix").cyan(),
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Fix application
// ---------------------------------------------------------------------------

/// Apply a single fix.
fn apply_fix(fix: &Fix, start_dir: &Path) -> anyhow::Result<()> {
    match fix {
        Fix::CreateManifest => {
            println!(
                "  {} Running {}...",
                style("\u{2192}").cyan().bold(),
                style("agx init -y").cyan()
            );
            super::init::run(super::init::InitArgs {
                force: false,
                scan_only: false,
                non_interactive: true,
                name: None,
                backend: Vec::new(),
            })?;
        }
        Fix::RunInstall => {
            println!(
                "  {} Running {}...",
                style("\u{2192}").cyan().bold(),
                style("agx install").cyan()
            );
            super::install::run(super::install::InstallArgs {})?;
        }
        Fix::RunGenerate => {
            println!(
                "  {} Running {}...",
                style("\u{2192}").cyan().bold(),
                style("agx generate").cyan()
            );
            super::generate::run(super::generate::GenerateArgs { backend: None })?;
        }
        Fix::InstallBinary { backend } => {
            let cmd = install_command(backend);
            if cmd.is_empty() {
                return Ok(());
            }
            println!(
                "  {} Running {}...",
                style("\u{2192}").cyan().bold(),
                style(&cmd).cyan()
            );
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!(
                        "  {} Installed successfully",
                        style("\u{2713}").green().bold()
                    );
                }
                Ok(s) => {
                    println!(
                        "  {} Install failed (exit code {})",
                        style("\u{2717}").red().bold(),
                        s.code().unwrap_or(-1)
                    );
                }
                Err(e) => {
                    println!(
                        "  {} Install failed: {e}",
                        style("\u{2717}").red().bold()
                    );
                }
            }
        }
        Fix::AddAgxToGitignore => {
            let gitignore_path = find_project_root(start_dir)
                .unwrap_or_else(|| start_dir.to_path_buf())
                .join(".gitignore");
            println!(
                "  {} Adding {} to .gitignore",
                style("\u{2192}").cyan().bold(),
                style(".agx/").cyan()
            );
            let mut content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(".agx/\n");
            std::fs::write(&gitignore_path, content)?;
        }
        Fix::TrackLockfile => {
            let project_root = find_project_root(start_dir)
                .unwrap_or_else(|| start_dir.to_path_buf());
            println!(
                "  {} Staging {} in git",
                style("\u{2192}").cyan().bold(),
                style("agent-environment.lock").cyan()
            );
            let status = std::process::Command::new("git")
                .args(["add", config::LOCKFILE_FILE])
                .current_dir(&project_root)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!(
                        "  {} Lockfile staged",
                        style("\u{2713}").green().bold()
                    );
                }
                _ => {
                    println!(
                        "  {} Failed to stage lockfile",
                        style("\u{2717}").red().bold()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Find the project root (directory containing the manifest) from a start dir.
fn find_project_root(start_dir: &Path) -> Option<PathBuf> {
    config::find_manifest(start_dir).and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

/// Binary name for a given backend.
fn backend_binary(name: &str) -> Option<&'static str> {
    match name {
        "claude-code" => Some("claude"),
        "gemini-cli" => Some("gemini"),
        "codex" => Some("codex"),
        // IDE backends have no CLI binary.
        "cursor" | "windsurf" | "generic" => None,
        _ => None,
    }
}

/// Install hint for missing binaries (human-readable).
fn install_hint(name: &str) -> &'static str {
    match name {
        "claude-code" => "npm install -g @anthropic-ai/claude-code",
        "gemini-cli" => "npm install -g @anthropic-ai/gemini-cli",
        "codex" => "npm install -g @openai/codex",
        _ => "",
    }
}

/// npm install command for a backend (machine-executable).
fn install_command(name: &str) -> &'static str {
    match name {
        "claude-code" => "npm install -g @anthropic-ai/claude-code",
        "gemini-cli" => "npm install -g @anthropic-ai/gemini-cli",
        "codex" => "npm install -g @openai/codex",
        _ => "",
    }
}

/// Generated output directories for each backend.
fn backend_output_dirs(name: &str) -> Vec<&'static str> {
    match name {
        "claude-code" => vec![".claude", ".mcp.json"],
        "gemini-cli" => vec![".gemini"],
        "codex" => vec![".codex"],
        "cursor" => vec![".cursor/rules"],
        "windsurf" => vec![".windsurf/rules"],
        "generic" => vec![".agents"],
        _ => Vec::new(),
    }
}

/// Find a binary on PATH, returning its absolute path.
fn find_binary(name: &str) -> Option<PathBuf> {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(trimmed))
                }
            } else {
                None
            }
        })
}

/// Check that `git` is on PATH.
fn check_git(report: &mut Report) {
    match find_binary("git") {
        Some(path) => report.pass(&format!(
            "git found ({})",
            style(path.display()).dim()
        )),
        None => {
            report.fail("git not found on PATH");
            report.detail("agx requires git for fetching dependencies");
        }
    }
}

/// Check backend binaries exist on PATH.
fn check_backend_binaries(env: &AgentEnvironment, report: &mut Report) {
    for name in env.backends.keys() {
        if let Some(binary) = backend_binary(name) {
            match find_binary(binary) {
                Some(path) => {
                    let version = get_binary_version(binary);
                    let version_str = version
                        .as_deref()
                        .map(|v| format!(", {v}"))
                        .unwrap_or_default();
                    report.pass(&format!(
                        "{binary} found ({}{})",
                        style(path.display()).dim(),
                        version_str,
                    ));
                }
                None => {
                    let hint = install_hint(name);
                    report.fail_fixable(
                        &format!("{binary} not found on PATH"),
                        Fix::InstallBinary {
                            backend: name.clone(),
                        },
                    );
                    if !hint.is_empty() {
                        report.detail(&format!("Install: {}", style(hint).cyan()));
                    }
                }
            }
        }
    }
}

/// Try to get a version string from a binary.
fn get_binary_version(binary: &str) -> Option<String> {
    std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                // Take just the first line, trimmed.
                Some(s.lines().next()?.trim().to_string())
            } else {
                None
            }
        })
}

/// Validate the manifest.
fn check_manifest_validity(env: &AgentEnvironment, report: &mut Report) {
    match manifest::validate::validate(env) {
        Ok(()) => report.pass("Manifest valid"),
        Err(errors) => {
            report.fail(&format!(
                "Manifest has {} error{}",
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            ));
            for err in &errors {
                report.detail(&err.to_string());
            }
        }
    }
}

/// Check lockfile existence and staleness.
fn check_lockfile(
    manifest_path: &Path,
    project_root: &Path,
    env: &AgentEnvironment,
    report: &mut Report,
) {
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    if !lockfile_path.exists() {
        report.warn_fixable(
            &format!(
                "No lockfile \u{2014} run {} to resolve dependencies",
                style("agx install").cyan(),
            ),
            Fix::RunInstall,
        );
        return;
    }

    // Staleness: mtime check.
    let manifest_mtime = std::fs::metadata(manifest_path)
        .and_then(|m| m.modified())
        .ok();
    let lockfile_mtime = std::fs::metadata(&lockfile_path)
        .and_then(|m| m.modified())
        .ok();

    if let (Some(m_time), Some(l_time)) = (manifest_mtime, lockfile_mtime) {
        if m_time > l_time {
            report.warn_fixable(
                &format!(
                    "Lockfile older than manifest \u{2014} run {} to update",
                    style("agx install").cyan(),
                ),
                Fix::RunInstall,
            );
            return;
        }
    }

    // Content staleness.
    match agx::lockfile::load(&lockfile_path) {
        Ok(lf) => {
            if agx::lockfile::is_stale(&lf, env) {
                report.warn_fixable(
                    &format!(
                        "Lockfile out of sync with manifest \u{2014} run {} to update",
                        style("agx install").cyan(),
                    ),
                    Fix::RunInstall,
                );
            } else {
                report.pass("Lockfile up to date");
            }
        }
        Err(e) => {
            report.fail(&format!("Lockfile failed to parse: {e}"));
        }
    }
}

/// Check cache integrity for locked entries that have integrity hashes.
fn check_cache_integrity(project_root: &Path, report: &mut Report) {
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    let lockfile = if lockfile_path.exists() {
        agx::lockfile::load(&lockfile_path).ok()
    } else {
        None
    };

    let cache_dir = match cache::cache_dir() {
        Ok(d) => d,
        Err(_) => {
            report.warn("Could not determine cache directory");
            return;
        }
    };

    if !cache_dir.exists() {
        report.pass("Cache: empty (no assets fetched yet)");
        return;
    }

    match cache::list_cached() {
        Ok(entries) => {
            if entries.is_empty() {
                report.pass("Cache: empty");
                return;
            }

            let total_size: u64 = entries.iter().map(|e| e.size_bytes).sum();

            // Verify integrity hashes if lockfile is available.
            let mut integrity_checked = 0usize;
            let mut integrity_failed = Vec::new();

            if let Some(ref lf) = lockfile {
                let all_entries = lf
                    .plugins
                    .iter()
                    .map(|(n, e)| ("plugins", n, e))
                    .chain(lf.skills.iter().map(|(n, e)| ("skills", n, e)))
                    .chain(lf.agents.iter().map(|(n, e)| ("agents", n, e)));

                for (category, name, entry) in all_entries {
                    if let Some(ref expected_hash) = entry.integrity {
                        let cached_path = find_cached_path(category, name, entry, &cache_dir);
                        if let Some(ref path) = cached_path {
                            integrity_checked += 1;
                            let ok = if path.is_dir() {
                                cache::integrity::verify_dir(path, expected_hash)
                                    .unwrap_or(false)
                            } else {
                                cache::integrity::verify_file(path, expected_hash)
                                    .unwrap_or(false)
                            };
                            if !ok {
                                integrity_failed.push(format!("{category}/{name}"));
                            }
                        }
                    }
                }
            }

            if integrity_failed.is_empty() {
                let integrity_msg = if integrity_checked > 0 {
                    format!(", {integrity_checked} integrity checks passed")
                } else {
                    String::new()
                };
                report.pass(&format!(
                    "Cache: {} entr{}, {}{}",
                    entries.len(),
                    if entries.len() == 1 { "y" } else { "ies" },
                    cache::format_size(total_size),
                    integrity_msg,
                ));
            } else {
                report.fail_fixable(
                    &format!(
                        "Cache: {} integrity check{} failed",
                        integrity_failed.len(),
                        if integrity_failed.len() == 1 { "" } else { "s" },
                    ),
                    Fix::RunInstall,
                );
                for name in &integrity_failed {
                    report.detail(&format!(
                        "{name} \u{2014} run {} to re-fetch",
                        style("agx install").cyan()
                    ));
                }
            }
        }
        Err(e) => {
            report.warn(&format!("Could not read cache: {e}"));
        }
    }
}

/// Resolve the cached path for a locked entry.
fn find_cached_path(
    category: &str,
    name: &str,
    entry: &agx::lockfile::types::LockedEntry,
    cache_dir: &Path,
) -> Option<PathBuf> {
    use agx::lockfile::types::LockedSource;

    let version_or_commit = match &entry.source {
        LockedSource::Registry {
            resolved_version, ..
        } => resolved_version.clone(),
        LockedSource::Git { .. } => entry.git_commit.clone()?,
        LockedSource::Npm {
            resolved_version, ..
        } => resolved_version.clone()?,
        LockedSource::Local { .. } => return None, // local paths aren't cached
    };

    let path = cache_dir
        .join(category)
        .join(name)
        .join(&version_or_commit);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Check that generated backend output directories exist.
fn check_generated_files(
    project_root: &Path,
    env: &AgentEnvironment,
    report: &mut Report,
) {
    // Only meaningful if a lockfile exists (otherwise generate can't run).
    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    if !lockfile_path.exists() {
        return;
    }

    let mut missing = Vec::new();
    for name in env.backends.keys() {
        for dir in backend_output_dirs(name) {
            let path = project_root.join(dir);
            if !path.exists() {
                missing.push(format!("{dir} ({name})"));
            }
        }
    }

    if missing.is_empty() {
        report.pass("Generated files present for all backends");
    } else {
        report.warn_fixable(
            &format!(
                "Missing generated output \u{2014} run {} to regenerate",
                style("agx generate").cyan(),
            ),
            Fix::RunGenerate,
        );
        for m in &missing {
            report.detail(m);
        }
    }
}

/// Check that `.agx/` is in `.gitignore` (if in a git repo).
fn check_gitignore(project_root: &Path, report: &mut Report) {
    // Only check if we're in a git repo.
    let git_dir = project_root.join(".git");
    if !git_dir.exists() {
        return;
    }

    let gitignore_path = project_root.join(".gitignore");
    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
        let has_agx = content
            .lines()
            .any(|line| {
                let trimmed = line.trim();
                trimmed == ".agx/" || trimmed == ".agx" || trimmed == "/.agx/" || trimmed == "/.agx"
            });
        if has_agx {
            report.pass(".agx/ in .gitignore");
        } else {
            report.warn_fixable(
                &format!(
                    ".agx/ not in .gitignore \u{2014} generated workspace files may be committed",
                ),
                Fix::AddAgxToGitignore,
            );
        }
    } else {
        report.warn_fixable(
            &format!(
                "No .gitignore found \u{2014} .agx/ workspace files may be committed",
            ),
            Fix::AddAgxToGitignore,
        );
    }
}

/// Check that `agent-environment.lock` is tracked in git.
fn check_lockfile_tracked(project_root: &Path, report: &mut Report) {
    let git_dir = project_root.join(".git");
    if !git_dir.exists() {
        return;
    }

    let lockfile_path = project_root.join(config::LOCKFILE_FILE);
    if !lockfile_path.exists() {
        return; // No lockfile to track yet.
    }

    // Check if the file is tracked by git.
    let output = std::process::Command::new("git")
        .args(["ls-files", config::LOCKFILE_FILE])
        .current_dir(project_root)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.trim().is_empty() {
                report.warn_fixable(
                    &format!(
                        "{} not tracked in git \u{2014} should be committed for reproducible environments",
                        config::LOCKFILE_FILE,
                    ),
                    Fix::TrackLockfile,
                );
            } else {
                report.pass(&format!("{} tracked in git", config::LOCKFILE_FILE));
            }
        }
        _ => {
            // Can't determine — skip silently.
        }
    }
}

/// Known user-level config paths for each backend.
fn user_config_paths(backend: &str) -> Vec<(&'static str, PathBuf)> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };

    match backend {
        "claude-code" => vec![
            ("~/.claude/CLAUDE.md", home.join(".claude/CLAUDE.md")),
            (
                "~/.claude/settings.json",
                home.join(".claude/settings.json"),
            ),
            ("~/.claude/plugins/", home.join(".claude/plugins")),
        ],
        "gemini-cli" => vec![(
            "~/.gemini/settings.json",
            home.join(".gemini/settings.json"),
        )],
        "codex" => vec![("~/.codex/", home.join(".codex"))],
        _ => Vec::new(),
    }
}

/// Check user-level configs and cross-reference with isolation mode.
fn check_user_config(
    backend: &str,
    isolation: &IsolationMode,
    env: &AgentEnvironment,
    report: &mut Report,
) {
    let paths = user_config_paths(backend);
    if paths.is_empty() {
        return;
    }

    let isolation_note = match isolation {
        IsolationMode::Strict => "blocked by strict isolation",
        IsolationMode::Bare => "blocked by bare isolation",
        IsolationMode::Permissive => "active \u{2014} layers into project",
    };

    for (label, path) in &paths {
        if path.exists() {
            let detail = summarize_config(label, path);
            let detail_str = if detail.is_empty() {
                String::new()
            } else {
                format!(" \u{2014} {detail}")
            };

            report.pass(&format!(
                "{label} found{detail_str} ({isolation_note})"
            ));

            // Check for MCP server name conflicts (claude-code specific).
            if backend == "claude-code" && *label == "~/.claude/settings.json" {
                check_mcp_conflicts(path, env, isolation, report);
            }
        }
    }

    // If none exist, say so.
    let any_exists = paths.iter().any(|(_, p)| p.exists());
    if !any_exists {
        report.pass("No user-level configuration found");
    }
}

/// Summarize a config file (counts of interesting entries).
fn summarize_config(label: &str, path: &Path) -> String {
    if label.ends_with("settings.json") {
        summarize_settings_json(path)
    } else if label.ends_with("plugins/") && path.is_dir() {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                let count = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .count();
                if count > 0 {
                    format!(
                        "{count} plugin{}",
                        if count == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    } else {
        String::new()
    }
}

/// Parse a settings.json and count permissions, MCP servers, and hooks.
fn summarize_settings_json(path: &Path) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let mut parts = Vec::new();

    // Count permissions (allowedTools, blockedTools, permissions).
    let mut perm_count = 0usize;
    for key in &["allowedTools", "blockedTools", "permissions"] {
        if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
            perm_count += arr.len();
        }
    }
    if perm_count > 0 {
        parts.push(format!(
            "{perm_count} permission{}",
            if perm_count == 1 { "" } else { "s" }
        ));
    }

    // Count MCP servers.
    if let Some(obj) = json.get("mcpServers").and_then(|v| v.as_object()) {
        let count = obj.len();
        if count > 0 {
            parts.push(format!(
                "{count} MCP server{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }

    // Count hooks.
    if let Some(obj) = json.get("hooks").and_then(|v| v.as_object()) {
        let count: usize = obj
            .values()
            .filter_map(|v| v.as_array())
            .map(|a| a.len())
            .sum();
        if count > 0 {
            parts.push(format!(
                "{count} hook{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }

    parts.join(", ")
}

/// Check for MCP server name conflicts between user settings and project manifest.
fn check_mcp_conflicts(
    settings_path: &Path,
    env: &AgentEnvironment,
    isolation: &IsolationMode,
    report: &mut Report,
) {
    if env.mcp_servers.is_empty() {
        return;
    }

    let content = match std::fs::read_to_string(settings_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(obj) = json.get("mcpServers").and_then(|v| v.as_object()) {
        for server_name in obj.keys() {
            if env.mcp_servers.contains_key(server_name) {
                match isolation {
                    IsolationMode::Permissive => {
                        report.warn(&format!(
                            "User MCP server \"{server_name}\" conflicts with project-declared server"
                        ));
                        report.detail(
                            "In permissive mode, the user server may shadow the project server",
                        );
                    }
                    IsolationMode::Strict | IsolationMode::Bare => {
                        report.pass(&format!(
                            "User MCP server \"{server_name}\" would conflict, but is blocked by {isolation} isolation"
                        ));
                    }
                }
            }
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
    fn run_in_with_valid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: test-doctor\nbackends:\n  claude-code:\n    isolation: strict\n",
        )
        .unwrap();

        let result = run_in(tmp.path(), false);
        assert!(result.is_ok());
    }

    #[test]
    fn run_in_no_manifest_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run_in(tmp.path(), false);
        assert!(result.is_ok());
    }

    #[test]
    fn run_in_invalid_manifest_reports_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "{{{{invalid yaml!!!!",
        )
        .unwrap();

        let result = run_in(tmp.path(), false);
        assert!(result.is_ok()); // doctor itself succeeds; it reports the error
    }

    #[test]
    fn summarize_settings_json_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "allowedTools": ["bash", "read"],
                "mcpServers": {"postgres": {}, "redis": {}},
                "hooks": {"PreToolUse": [{"command": "echo"}]}
            }"#,
        )
        .unwrap();

        let summary = summarize_settings_json(&path);
        assert!(summary.contains("2 permissions"));
        assert!(summary.contains("2 MCP servers"));
        assert!(summary.contains("1 hook"));
    }

    #[test]
    fn summarize_settings_json_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();

        let summary = summarize_settings_json(&path);
        assert!(summary.is_empty());
    }

    #[test]
    fn summarize_settings_json_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "not json").unwrap();

        let summary = summarize_settings_json(&path);
        assert!(summary.is_empty());
    }

    #[test]
    fn backend_binary_names() {
        assert_eq!(backend_binary("claude-code"), Some("claude"));
        assert_eq!(backend_binary("gemini-cli"), Some("gemini"));
        assert_eq!(backend_binary("codex"), Some("codex"));
        assert_eq!(backend_binary("cursor"), None);
        assert_eq!(backend_binary("windsurf"), None);
        assert_eq!(backend_binary("generic"), None);
        assert_eq!(backend_binary("unknown"), None);
    }

    #[test]
    fn check_manifest_validity_pass() {
        let env = minimal_env();
        let mut report = Report::default();
        check_manifest_validity(&env, &mut report);
        assert_eq!(report.pass, 1);
        assert_eq!(report.fail, 0);
    }

    #[test]
    fn check_lockfile_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("agent-environment.yml");
        std::fs::write(&manifest_path, "name: t\nbackends:\n  claude-code: {}\n").unwrap();
        let env = minimal_env();
        let mut report = Report::default();
        check_lockfile(&manifest_path, tmp.path(), &env, &mut report);
        assert_eq!(report.warn, 1);
        assert!(report.fixes.contains(&Fix::RunInstall));
    }

    #[test]
    fn user_config_paths_claude_code() {
        let paths = user_config_paths("claude-code");
        assert_eq!(paths.len(), 3);
        assert!(paths[0].0.contains("CLAUDE.md"));
        assert!(paths[1].0.contains("settings.json"));
        assert!(paths[2].0.contains("plugins"));
    }

    #[test]
    fn user_config_paths_unknown() {
        let paths = user_config_paths("unknown-backend");
        assert!(paths.is_empty());
    }

    #[test]
    fn mcp_conflict_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"mcpServers": {"postgres": {"command": "pg"}}}"#,
        )
        .unwrap();

        let mut env = minimal_env();
        env.mcp_servers.insert(
            "postgres".to_string(),
            agx::manifest::types::McpServerConfig {
                url: None,
                command: Some("pg-server".to_string()),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
        );

        // Permissive: should warn.
        let mut report = Report::default();
        check_mcp_conflicts(&settings_path, &env, &IsolationMode::Permissive, &mut report);
        assert_eq!(report.warn, 1);

        // Strict: should pass (blocked).
        let mut report = Report::default();
        check_mcp_conflicts(&settings_path, &env, &IsolationMode::Strict, &mut report);
        assert_eq!(report.pass, 1);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn mcp_no_conflict_when_no_project_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"mcpServers": {"postgres": {}}}"#,
        )
        .unwrap();

        let env = minimal_env(); // no mcp_servers
        let mut report = Report::default();
        check_mcp_conflicts(&settings_path, &env, &IsolationMode::Permissive, &mut report);
        assert_eq!(report.warn, 0);
        assert_eq!(report.pass, 0);
    }

    #[test]
    fn report_summary_no_problems() {
        let report = Report {
            pass: 5,
            warn: 0,
            fail: 0,
            fixes: Vec::new(),
        };
        report.print_summary();
    }

    #[test]
    fn report_summary_with_warnings() {
        let report = Report {
            pass: 3,
            warn: 2,
            fail: 0,
            fixes: Vec::new(),
        };
        report.print_summary();
    }

    #[test]
    fn report_summary_with_failures() {
        let report = Report {
            pass: 1,
            warn: 1,
            fail: 2,
            fixes: Vec::new(),
        };
        report.print_summary();
    }

    #[test]
    fn run_from_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: cwd-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = run(DoctorArgs { fix: false });
        std::env::set_current_dir(&original_dir).unwrap();
        drop(_guard);

        assert!(result.is_ok());
    }

    #[test]
    fn run_in_with_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: subdir-test\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        let sub = tmp.path().join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();

        let result = run_in(&sub, false);
        assert!(result.is_ok());
    }

    #[test]
    fn run_in_with_multiple_backends() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: multi\nbackends:\n  claude-code:\n    isolation: strict\n  gemini-cli:\n    isolation: permissive\n",
        )
        .unwrap();

        let result = run_in(tmp.path(), false);
        assert!(result.is_ok());
    }

    // --- Fix-related tests ---

    #[test]
    fn fix_deduplicates_install_and_generate() {
        let mut report = Report::default();
        report.fixes.push(Fix::RunInstall);
        report.fixes.push(Fix::RunGenerate);
        report.fixes.push(Fix::AddAgxToGitignore);

        let mut fixes = report.fixes.clone();
        if fixes.contains(&Fix::RunInstall) {
            fixes.retain(|f| !matches!(f, Fix::RunGenerate));
        }

        assert_eq!(fixes.len(), 2);
        assert!(fixes.contains(&Fix::RunInstall));
        assert!(fixes.contains(&Fix::AddAgxToGitignore));
        assert!(!fixes.contains(&Fix::RunGenerate));
    }

    #[test]
    fn check_gitignore_detects_missing_agx() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a git repo.
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\n").unwrap();

        let mut report = Report::default();
        check_gitignore(tmp.path(), &mut report);
        assert_eq!(report.warn, 1);
        assert!(report.fixes.contains(&Fix::AddAgxToGitignore));
    }

    #[test]
    fn check_gitignore_passes_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\n.agx/\n").unwrap();

        let mut report = Report::default();
        check_gitignore(tmp.path(), &mut report);
        assert_eq!(report.pass, 1);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn check_gitignore_skipped_outside_git() {
        let tmp = tempfile::tempdir().unwrap();
        // No .git directory.

        let mut report = Report::default();
        check_gitignore(tmp.path(), &mut report);
        assert_eq!(report.pass, 0);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn check_lockfile_tracked_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: t\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        let lockfile = agx::lockfile::types::Lockfile::new();
        agx::lockfile::save(&lockfile, &tmp.path().join("agent-environment.lock")).unwrap();

        let mut report = Report::default();
        check_lockfile_tracked(tmp.path(), &mut report);
        // In a fake .git dir, git ls-files will fail — that's fine, we just skip.
        // The test verifies we don't crash.
    }

    #[test]
    fn check_lockfile_tracked_skipped_without_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        let mut report = Report::default();
        check_lockfile_tracked(tmp.path(), &mut report);
        assert_eq!(report.pass, 0);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn check_generated_files_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: t\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        // Create a lockfile so the check is relevant.
        let lockfile = agx::lockfile::types::Lockfile::new();
        agx::lockfile::save(&lockfile, &tmp.path().join("agent-environment.lock")).unwrap();

        let env = minimal_env();
        let mut report = Report::default();
        check_generated_files(tmp.path(), &env, &mut report);
        assert_eq!(report.warn, 1);
        assert!(report.fixes.contains(&Fix::RunGenerate));
    }

    #[test]
    fn check_generated_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: t\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        let lockfile = agx::lockfile::types::Lockfile::new();
        agx::lockfile::save(&lockfile, &tmp.path().join("agent-environment.lock")).unwrap();

        // Create the expected output dirs.
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(tmp.path().join(".mcp.json"), "{}").unwrap();

        let env = minimal_env();
        let mut report = Report::default();
        check_generated_files(tmp.path(), &env, &mut report);
        assert_eq!(report.pass, 1);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn check_generated_files_skipped_without_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        // No lockfile.

        let env = minimal_env();
        let mut report = Report::default();
        check_generated_files(tmp.path(), &env, &mut report);
        assert_eq!(report.pass, 0);
        assert_eq!(report.warn, 0);
    }

    #[test]
    fn fix_add_agx_to_gitignore_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: t\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();

        apply_fix(&Fix::AddAgxToGitignore, tmp.path()).unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains(".agx/"));
    }

    #[test]
    fn fix_add_agx_to_gitignore_appends() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-environment.yml"),
            "name: t\nbackends:\n  claude-code: {}\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\n").unwrap();

        apply_fix(&Fix::AddAgxToGitignore, tmp.path()).unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(".agx/"));
    }

    #[test]
    fn no_manifest_queues_create_fix() {
        let tmp = tempfile::tempdir().unwrap();
        let mut report = Report::default();

        // Simulate the no-manifest path.
        report.warn_fixable("No manifest", Fix::CreateManifest);
        assert!(report.fixes.contains(&Fix::CreateManifest));
    }

    #[test]
    fn backend_output_dirs_known() {
        assert!(!backend_output_dirs("claude-code").is_empty());
        assert!(!backend_output_dirs("gemini-cli").is_empty());
        assert!(!backend_output_dirs("codex").is_empty());
        assert!(!backend_output_dirs("cursor").is_empty());
        assert!(!backend_output_dirs("windsurf").is_empty());
        assert!(!backend_output_dirs("generic").is_empty());
        assert!(backend_output_dirs("unknown").is_empty());
    }

    #[test]
    fn install_command_matches_hint() {
        // install_command and install_hint should agree on the npm package.
        for backend in &["claude-code", "codex"] {
            assert_eq!(install_command(backend), install_hint(backend));
        }
    }
}
