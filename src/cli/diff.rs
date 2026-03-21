//! `agx diff` — show what would change if `agx generate` were run now.

use agx::backends::{get_backend, ResolvedEnvironment, GENERATED_MARKER};
use agx::context;
use agx::lockfile::types::Lockfile;
use anyhow::{bail, Context};
use clap::Args;
use console::style;
use std::path::{Path, PathBuf};

/// Arguments for `agx diff`.
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Only check a specific backend.
    #[arg(long)]
    pub backend: Option<String>,
}

/// Maximum number of diff lines to show per file before truncating.
const MAX_DIFF_LINES_PER_FILE: usize = 20;

/// Result of comparing two sets of files.
#[derive(Debug, Default)]
struct DiffSummary {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

impl DiffSummary {
    /// Returns true if there are any differences.
    fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.modified.is_empty() || !self.deleted.is_empty()
    }
}

/// Run the `agx diff` command.
///
/// Generates backend and context files to a temporary directory, then compares
/// them against the actual project files. Prints a summary of differences and
/// exits with code 1 if any differences are found.
pub fn run(args: DiffArgs) -> anyhow::Result<()> {
    let ctx = super::ProjectContext::load()?;
    let manifest = &ctx.manifest;
    let project_root = &ctx.project_root;
    let lockfile = ctx.lockfile.unwrap_or_else(Lockfile::new);

    // Determine cache directory.
    let cache_dir = agx::cache::cache_dir()?;

    let resolved = ResolvedEnvironment {
        manifest,
        lockfile: &lockfile,
        cache_dir: &cache_dir,
    };

    // Filter backends if --backend was specified.
    let backend_names: Vec<&String> = if let Some(ref name) = args.backend {
        if !manifest.backends.contains_key(name) {
            bail!(
                "backend '{}' is not declared in the manifest; available: {:?}",
                name,
                manifest.backends.keys().collect::<Vec<_>>()
            );
        }
        vec![manifest.backends.keys().find(|k| *k == name).unwrap()]
    } else {
        manifest.backends.keys().collect()
    };

    // Generate to a tempdir.
    let tempdir = tempfile::tempdir().context("creating temporary directory")?;
    let temp_path = tempdir.path();

    for name in &backend_names {
        let backend = get_backend(name).ok_or_else(|| {
            anyhow::anyhow!(
                "no built-in backend for '{}'; available: claude-code, gemini-cli, generic",
                name
            )
        })?;

        backend
            .generate(&resolved, temp_path)
            .with_context(|| format!("generating {} backend to tempdir", backend.name()))?;
    }

    // Generate context files to tempdir.
    context::generate_context_files(manifest, temp_path)
        .context("generating context files to tempdir")?;

    // Compare tempdir against project root.
    let summary = compare_trees(temp_path, project_root)?;

    // Print results.
    if !summary.has_changes() {
        eprintln!("{}", style("no differences found.").dim());
        return Ok(());
    }

    for path in &summary.added {
        println!("{}", style(format!("+ {path}")).green());
    }

    for path in &summary.modified {
        println!("{}", style(format!("~ {path}")).yellow());
        // Show a simple diff for modified files.
        let temp_content = std::fs::read_to_string(temp_path.join(path))?;
        let project_content = std::fs::read_to_string(project_root.join(path))?;
        print_simple_diff(&project_content, &temp_content);
    }

    for path in &summary.deleted {
        println!("{}", style(format!("- {path}")).red());
    }

    let total = summary.added.len() + summary.modified.len() + summary.deleted.len();
    eprintln!(
        "\n{} file(s) differ: {} added, {} modified, {} deleted",
        total,
        summary.added.len(),
        summary.modified.len(),
        summary.deleted.len()
    );

    std::process::exit(1);
}

/// Compare two directory trees and return a summary of differences.
///
/// Files in `temp_dir` that don't exist in `project_dir` are "added".
/// Files that exist in both but differ are "modified".
/// Files in `project_dir` with the agx marker that don't exist in `temp_dir` are "deleted".
fn compare_trees(temp_dir: &Path, project_dir: &Path) -> anyhow::Result<DiffSummary> {
    let mut summary = DiffSummary::default();

    // Walk the tempdir to find added/modified files.
    for entry in walkdir::WalkDir::new(temp_dir).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(temp_dir)
            .expect("path must be under tempdir");
        let relative_str = relative.to_string_lossy().to_string();
        let project_file = project_dir.join(relative);

        if !project_file.exists() {
            summary.added.push(relative_str);
        } else {
            let temp_content = std::fs::read_to_string(entry.path())
                .with_context(|| format!("reading temp file {}", entry.path().display()))?;
            let project_content = std::fs::read_to_string(&project_file)
                .with_context(|| format!("reading project file {}", project_file.display()))?;

            if temp_content != project_content {
                summary.modified.push(relative_str);
            }
        }
    }

    // Walk the project dir to find deleted files (files with agx marker that
    // are no longer generated).
    // Check known generated locations rather than walking the entire project.
    let generated_patterns = collect_generated_files(project_dir)?;
    for file_path in generated_patterns {
        let relative = file_path
            .strip_prefix(project_dir)
            .expect("path must be under project dir");
        let temp_file = temp_dir.join(relative);
        if !temp_file.exists() {
            summary.deleted.push(relative.to_string_lossy().to_string());
        }
    }

    // Sort for deterministic output.
    summary.added.sort();
    summary.modified.sort();
    summary.deleted.sort();

    Ok(summary)
}

/// Collect files in the project directory that contain the agx generated marker.
/// Only checks known generated locations to avoid scanning the entire tree.
fn collect_generated_files(project_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    // Check known generated file paths.
    let candidates = [
        ".mcp.json",
        ".claude/settings.json",
        "CLAUDE.md",
        "GEMINI.md",
        "AGENTS.md",
    ];

    for candidate in &candidates {
        let path = project_dir.join(candidate);
        if path.exists() && file_has_agx_marker(&path)? {
            files.push(path);
        }
    }

    // Walk .claude/agents/ and .claude/skills/ if they exist.
    let agents_dir = project_dir.join(".claude").join("agents");
    if agents_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&agents_dir).min_depth(1) {
            let entry = entry?;
            if entry.file_type().is_file() && file_has_agx_marker(entry.path())? {
                files.push(entry.path().to_path_buf());
            }
        }
    }

    let skills_dir = project_dir.join(".claude").join("skills");
    if skills_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&skills_dir).min_depth(1) {
            let entry = entry?;
            if entry.file_type().is_file() && file_has_agx_marker(entry.path())? {
                files.push(entry.path().to_path_buf());
            }
        }
    }

    // Also check Gemini CLI paths.
    let gemini_dir = project_dir.join(".gemini");
    if gemini_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&gemini_dir).min_depth(1) {
            let entry = entry?;
            if entry.file_type().is_file() && file_has_agx_marker(entry.path())? {
                files.push(entry.path().to_path_buf());
            }
        }
    }

    Ok(files)
}

/// Check if a file contains the agx generated marker or the auto-generated comment.
fn file_has_agx_marker(path: &Path) -> anyhow::Result<bool> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(content.contains(GENERATED_MARKER) || content.contains("AUTO-GENERATED BY agx"))
}

/// Print a simple line-by-line diff between old and new content.
///
/// Shows lines prefixed with `-` (removed) and `+` (added), limited to
/// `MAX_DIFF_LINES_PER_FILE` lines per file.
fn print_simple_diff(old: &str, new: &str) {
    let diff_lines = compute_simple_diff(old, new);

    let total = diff_lines.len();
    let display_count = total.min(MAX_DIFF_LINES_PER_FILE);

    for line in &diff_lines[..display_count] {
        if let Some(stripped) = line.strip_prefix('+') {
            println!("  {}", style(format!("+{stripped}")).green());
        } else if let Some(stripped) = line.strip_prefix('-') {
            println!("  {}", style(format!("-{stripped}")).red());
        } else {
            println!("  {line}");
        }
    }

    if total > MAX_DIFF_LINES_PER_FILE {
        println!(
            "  {}",
            style(format!("... and {} more lines", total - MAX_DIFF_LINES_PER_FILE)).dim()
        );
    }
}

/// Compute a simple line-level diff between old and new content.
///
/// Returns lines prefixed with `+` (added), `-` (removed), or ` ` (unchanged).
/// Uses a straightforward approach: finds common lines and marks differences.
fn compute_simple_diff(old: &str, new: &str) -> Vec<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut result = Vec::new();

    // Simple LCS-based diff using a two-pointer approach on the edit script.
    let lcs = longest_common_subsequence(&old_lines, &new_lines);

    let mut old_idx = 0;
    let mut new_idx = 0;

    for (oi, ni) in &lcs {
        // Lines removed from old before this common line.
        while old_idx < *oi {
            result.push(format!("-{}", old_lines[old_idx]));
            old_idx += 1;
        }
        // Lines added in new before this common line.
        while new_idx < *ni {
            result.push(format!("+{}", new_lines[new_idx]));
            new_idx += 1;
        }
        // Common line — skip (don't include context for brevity).
        old_idx = oi + 1;
        new_idx = ni + 1;
    }

    // Remaining lines after last common line.
    while old_idx < old_lines.len() {
        result.push(format!("-{}", old_lines[old_idx]));
        old_idx += 1;
    }
    while new_idx < new_lines.len() {
        result.push(format!("+{}", new_lines[new_idx]));
        new_idx += 1;
    }

    result
}

/// Compute the longest common subsequence of two slices of lines.
/// Returns pairs of (old_index, new_index) for each matched line.
fn longest_common_subsequence<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(usize, usize)> {
    let m = old.len();
    let n = new.len();

    // Build DP table.
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack to find the pairs.
    let mut result = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 && j > 0 {
        if old[i - 1] == new[j - 1] {
            result.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    result.reverse();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_compute_simple_diff_no_changes() {
        let text = "line 1\nline 2\nline 3\n";
        let diff = compute_simple_diff(text, text);
        assert!(diff.is_empty(), "expected no diff lines, got: {:?}", diff);
    }

    #[test]
    fn test_compute_simple_diff_added_lines() {
        let old = "line 1\nline 3\n";
        let new = "line 1\nline 2\nline 3\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["+line 2"]);
    }

    #[test]
    fn test_compute_simple_diff_removed_lines() {
        let old = "line 1\nline 2\nline 3\n";
        let new = "line 1\nline 3\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-line 2"]);
    }

    #[test]
    fn test_compute_simple_diff_modified_lines() {
        let old = "line 1\nold line\nline 3\n";
        let new = "line 1\nnew line\nline 3\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-old line", "+new line"]);
    }

    #[test]
    fn test_compute_simple_diff_completely_different() {
        let old = "aaa\nbbb\n";
        let new = "ccc\nddd\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-aaa", "-bbb", "+ccc", "+ddd"]);
    }

    #[test]
    fn test_lcs_identical() {
        let lines = vec!["a", "b", "c"];
        let lcs = longest_common_subsequence(&lines, &lines);
        assert_eq!(lcs, vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn test_lcs_empty() {
        let old: Vec<&str> = vec![];
        let new: Vec<&str> = vec!["a"];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_lcs_partial_overlap() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "x", "c"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(0, 0), (2, 2)]);
    }

    #[test]
    fn test_compare_trees_no_differences() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Create identical files in both.
        fs::create_dir_all(temp.path().join("subdir")).unwrap();
        fs::create_dir_all(project.path().join("subdir")).unwrap();
        fs::write(temp.path().join("subdir/file.txt"), "content").unwrap();
        fs::write(project.path().join("subdir/file.txt"), "content").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(!summary.has_changes());
    }

    #[test]
    fn test_compare_trees_new_file() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // File exists only in tempdir.
        fs::write(temp.path().join("new-file.txt"), "new content").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert_eq!(summary.added, vec!["new-file.txt"]);
        assert!(summary.modified.is_empty());
        assert!(summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_modified_file() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Same file, different content.
        fs::write(temp.path().join("file.txt"), "new content").unwrap();
        fs::write(project.path().join("file.txt"), "old content").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(summary.added.is_empty());
        assert_eq!(summary.modified, vec!["file.txt"]);
        assert!(summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_deleted_file() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // File exists in project with agx marker, but not in tempdir.
        let marker_content = format!("{{ \"_generated_by\": \"{}\" }}", GENERATED_MARKER);
        fs::write(project.path().join(".mcp.json"), marker_content).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(summary.added.is_empty());
        assert!(summary.modified.is_empty());
        assert_eq!(summary.deleted, vec![".mcp.json"]);
    }

    #[test]
    fn test_file_has_agx_marker_true() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");
        let content = format!("{{ \"_generated_by\": \"{}\" }}", GENERATED_MARKER);
        fs::write(&path, content).unwrap();

        assert!(file_has_agx_marker(&path).unwrap());
    }

    #[test]
    fn test_file_has_agx_marker_auto_generated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.md");
        fs::write(&path, "<!-- AUTO-GENERATED BY agx. Source: local -->").unwrap();

        assert!(file_has_agx_marker(&path).unwrap());
    }

    #[test]
    fn test_file_has_agx_marker_false() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "just regular content").unwrap();

        assert!(!file_has_agx_marker(&path).unwrap());
    }

    #[test]
    fn test_diff_summary_has_changes() {
        let empty = DiffSummary::default();
        assert!(!empty.has_changes());

        let with_added = DiffSummary {
            added: vec!["file.txt".to_string()],
            ..Default::default()
        };
        assert!(with_added.has_changes());

        let with_modified = DiffSummary {
            modified: vec!["file.txt".to_string()],
            ..Default::default()
        };
        assert!(with_modified.has_changes());

        let with_deleted = DiffSummary {
            deleted: vec!["file.txt".to_string()],
            ..Default::default()
        };
        assert!(with_deleted.has_changes());
    }

    // -----------------------------------------------------------------------
    // Additional compute_simple_diff tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_simple_diff_empty_old() {
        let old = "";
        let new = "line 1\nline 2\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["+line 1", "+line 2"]);
    }

    #[test]
    fn test_compute_simple_diff_empty_new() {
        let old = "line 1\nline 2\n";
        let new = "";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-line 1", "-line 2"]);
    }

    #[test]
    fn test_compute_simple_diff_both_empty() {
        let diff = compute_simple_diff("", "");
        assert!(diff.is_empty());
    }

    #[test]
    fn test_compute_simple_diff_only_additions_at_end() {
        let old = "a\nb\n";
        let new = "a\nb\nc\nd\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["+c", "+d"]);
    }

    #[test]
    fn test_compute_simple_diff_only_removals_at_end() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nb\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-c", "-d"]);
    }

    #[test]
    fn test_compute_simple_diff_interleaved_changes() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nX\nc\nY\ne\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-b", "+X", "-d", "+Y"]);
    }

    #[test]
    fn test_compute_simple_diff_single_line_change() {
        let old = "only line";
        let new = "different line";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["-only line", "+different line"]);
    }

    // -----------------------------------------------------------------------
    // Additional LCS tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_lcs_both_empty() {
        let old: Vec<&str> = vec![];
        let new: Vec<&str> = vec![];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_lcs_no_common_elements() {
        let old = vec!["a", "b", "c"];
        let new = vec!["x", "y", "z"];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_lcs_one_element_match() {
        let old = vec!["a", "b", "c"];
        let new = vec!["x", "b", "z"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(1, 1)]);
    }

    #[test]
    fn test_lcs_subsequence_not_substring() {
        let old = vec!["a", "b", "c", "d"];
        let new = vec!["a", "c", "d"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(0, 0), (2, 1), (3, 2)]);
    }

    #[test]
    fn test_lcs_old_empty_new_nonempty() {
        let old: Vec<&str> = vec![];
        let new = vec!["a", "b"];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_lcs_old_nonempty_new_empty() {
        let old = vec!["a", "b"];
        let new: Vec<&str> = vec![];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Additional compare_trees tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_trees_multiple_changes() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Added file
        fs::write(temp.path().join("added.txt"), "new").unwrap();
        // Modified file
        fs::write(temp.path().join("shared.txt"), "new content").unwrap();
        fs::write(project.path().join("shared.txt"), "old content").unwrap();
        // Deleted file with agx marker
        let marker_content = format!("content with {}", GENERATED_MARKER);
        fs::write(project.path().join(".mcp.json"), marker_content).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert_eq!(summary.added, vec!["added.txt"]);
        assert_eq!(summary.modified, vec!["shared.txt"]);
        assert_eq!(summary.deleted, vec![".mcp.json"]);
        assert!(summary.has_changes());
    }

    #[test]
    fn test_compare_trees_nested_directories() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Create nested file in temp only
        fs::create_dir_all(temp.path().join("a/b")).unwrap();
        fs::write(temp.path().join("a/b/deep.txt"), "deep content").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert_eq!(summary.added, vec!["a/b/deep.txt"]);
    }

    #[test]
    fn test_compare_trees_both_empty() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(!summary.has_changes());
    }

    #[test]
    fn test_compare_trees_identical_content() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        fs::write(temp.path().join("same.txt"), "identical").unwrap();
        fs::write(project.path().join("same.txt"), "identical").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(!summary.has_changes());
    }

    // -----------------------------------------------------------------------
    // Additional file_has_agx_marker tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_has_agx_marker_embedded_in_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");
        let content = format!(
            "{{\n  \"_generated_by\": \"{}\",\n  \"key\": \"value\"\n}}",
            GENERATED_MARKER
        );
        fs::write(&path, content).unwrap();
        assert!(file_has_agx_marker(&path).unwrap());
    }

    #[test]
    fn test_file_has_agx_marker_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "").unwrap();
        assert!(!file_has_agx_marker(&path).unwrap());
    }

    // -----------------------------------------------------------------------
    // collect_generated_files tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_generated_files_none() {
        let dir = tempdir().unwrap();
        let files = collect_generated_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_generated_files_with_marker() {
        let dir = tempdir().unwrap();
        let marker_content = format!("some prefix {} some suffix", GENERATED_MARKER);
        fs::write(dir.path().join(".mcp.json"), &marker_content).unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "<!-- AUTO-GENERATED BY agx. Source: local -->").unwrap();
        // File without marker should not be collected
        fs::write(dir.path().join("GEMINI.md"), "not generated").unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with(".mcp.json")));
        assert!(files.iter().any(|p| p.ends_with("CLAUDE.md")));
    }

    #[test]
    fn test_collect_generated_files_claude_agents_dir() {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join(".claude").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let marker_content = format!("content with {}", GENERATED_MARKER);
        fs::write(agents_dir.join("reviewer.md"), &marker_content).unwrap();
        fs::write(agents_dir.join("plain.md"), "no marker").unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("reviewer.md"));
    }

    #[test]
    fn test_collect_generated_files_claude_skills_dir() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join(".claude").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        let marker_content = format!("content with {}", GENERATED_MARKER);
        fs::write(skills_dir.join("deploy.md"), &marker_content).unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("deploy.md"));
    }

    #[test]
    fn test_collect_generated_files_gemini_dir() {
        let dir = tempdir().unwrap();
        let gemini_dir = dir.path().join(".gemini");
        fs::create_dir_all(&gemini_dir).unwrap();
        let marker_content = format!("<!-- AUTO-GENERATED BY agx. Source: local -->");
        fs::write(gemini_dir.join("settings.json"), &marker_content).unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("settings.json"));
    }

    #[test]
    fn test_collect_generated_files_claude_settings() {
        let dir = tempdir().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let marker_content = format!("{{ \"_generated_by\": \"{}\" }}", GENERATED_MARKER);
        fs::write(claude_dir.join("settings.json"), &marker_content).unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("settings.json"));
    }

    // -----------------------------------------------------------------------
    // DiffSummary additional tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_summary_all_three_changes() {
        let summary = DiffSummary {
            added: vec!["a.txt".to_string()],
            modified: vec!["b.txt".to_string()],
            deleted: vec!["c.txt".to_string()],
        };
        assert!(summary.has_changes());
    }

    // -----------------------------------------------------------------------
    // print_simple_diff tests (exercised for coverage)
    // -----------------------------------------------------------------------

    #[test]
    fn test_print_simple_diff_no_changes() {
        let text = "line 1\nline 2\n";
        // Just exercise the function — it prints to stdout
        print_simple_diff(text, text);
    }

    #[test]
    fn test_print_simple_diff_additions() {
        let old = "a\nc\n";
        let new = "a\nb\nc\n";
        print_simple_diff(old, new);
    }

    #[test]
    fn test_print_simple_diff_removals() {
        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        print_simple_diff(old, new);
    }

    #[test]
    fn test_print_simple_diff_modifications() {
        let old = "a\nold\nc\n";
        let new = "a\nnew\nc\n";
        print_simple_diff(old, new);
    }

    #[test]
    fn test_print_simple_diff_truncation() {
        // Create a diff with more lines than MAX_DIFF_LINES_PER_FILE
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        for i in 0..30 {
            old_lines.push(format!("old line {}", i));
            new_lines.push(format!("new line {}", i));
        }
        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        // This should trigger the truncation path
        print_simple_diff(&old, &new);
    }

    #[test]
    fn test_print_simple_diff_context_lines() {
        // Diff with unchanged context lines (the " " prefix path)
        let old = "unchanged\nold\nunchanged2\n";
        let new = "unchanged\nnew\nunchanged2\n";
        print_simple_diff(old, new);
    }

    // -----------------------------------------------------------------------
    // compare_trees: more edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_trees_deleted_claude_agents_dir() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Create .claude/agents/ with marked files in project
        let agents_dir = project.path().join(".claude").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let marker_content = format!("content with {}", GENERATED_MARKER);
        fs::write(agents_dir.join("reviewer.md"), &marker_content).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(summary.has_changes());
        assert!(!summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_deleted_claude_skills_dir() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        let skills_dir = project.path().join(".claude").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        let marker_content = format!("content with {}", GENERATED_MARKER);
        fs::write(skills_dir.join("deploy.md"), &marker_content).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(!summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_deleted_gemini_dir() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        let gemini_dir = project.path().join(".gemini");
        fs::create_dir_all(&gemini_dir).unwrap();
        let marker_content = format!("<!-- AUTO-GENERATED BY agx. Source: local -->");
        fs::write(gemini_dir.join("settings.json"), &marker_content).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(!summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_non_marker_file_not_deleted() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // File in project without agx marker should NOT be listed as deleted
        fs::write(project.path().join("CLAUDE.md"), "regular content without marker").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert!(summary.deleted.is_empty());
    }

    #[test]
    fn test_compare_trees_agents_md_deleted() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        let marker = format!("<!-- AUTO-GENERATED BY agx. Source: local -->");
        fs::write(project.path().join("AGENTS.md"), &marker).unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert_eq!(summary.deleted, vec!["AGENTS.md"]);
    }

    // -----------------------------------------------------------------------
    // collect_generated_files: more edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_generated_files_mixed_marker_types() {
        let dir = tempdir().unwrap();
        let marker_content = format!("content {}", GENERATED_MARKER);
        let auto_marker = "<!-- AUTO-GENERATED BY agx. Source: local -->";

        // Both marker types
        fs::write(dir.path().join(".mcp.json"), &marker_content).unwrap();
        fs::write(dir.path().join("CLAUDE.md"), auto_marker).unwrap();
        // Non-marker file
        fs::write(dir.path().join("GEMINI.md"), "no marker here").unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_collect_generated_files_all_candidate_files() {
        let dir = tempdir().unwrap();
        let marker = format!("{}", GENERATED_MARKER);

        // Create all candidate files with marker
        fs::write(dir.path().join(".mcp.json"), &marker).unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(dir.path().join(".claude/settings.json"), &marker).unwrap();
        fs::write(dir.path().join("CLAUDE.md"), &marker).unwrap();
        fs::write(dir.path().join("GEMINI.md"), &marker).unwrap();
        fs::write(dir.path().join("AGENTS.md"), &marker).unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 5);
    }

    #[test]
    fn test_collect_generated_files_agents_dir_ignores_non_marker() {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join(".claude").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let marker = format!("{}", GENERATED_MARKER);
        fs::write(agents_dir.join("marked.md"), &marker).unwrap();
        fs::write(agents_dir.join("plain.md"), "no marker").unwrap();

        let files = collect_generated_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("marked.md"));
    }

    // -----------------------------------------------------------------------
    // compute_simple_diff: more patterns
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_simple_diff_reordered_lines() {
        let old = "a\nb\nc\n";
        let new = "c\nb\na\n";
        let diff = compute_simple_diff(old, new);
        // a and c are swapped, b stays
        assert!(!diff.is_empty());
    }

    #[test]
    fn test_compute_simple_diff_duplicate_lines() {
        let old = "a\na\nb\n";
        let new = "a\nb\nb\n";
        let diff = compute_simple_diff(old, new);
        assert!(!diff.is_empty());
    }

    #[test]
    fn test_compute_simple_diff_added_at_start() {
        let old = "b\nc\n";
        let new = "a\nb\nc\n";
        let diff = compute_simple_diff(old, new);
        assert_eq!(diff, vec!["+a"]);
    }

    // -----------------------------------------------------------------------
    // LCS: additional patterns
    // -----------------------------------------------------------------------

    #[test]
    fn test_lcs_single_element_both() {
        let old = vec!["a"];
        let new = vec!["a"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(0, 0)]);
    }

    #[test]
    fn test_lcs_single_element_no_match() {
        let old = vec!["a"];
        let new = vec!["b"];
        let lcs = longest_common_subsequence(&old, &new);
        assert!(lcs.is_empty());
    }

    #[test]
    fn test_lcs_prefix_overlap() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "b", "d"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn test_lcs_suffix_overlap() {
        let old = vec!["x", "b", "c"];
        let new = vec!["y", "b", "c"];
        let lcs = longest_common_subsequence(&old, &new);
        assert_eq!(lcs, vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn test_compare_trees_results_sorted() {
        let temp = tempdir().unwrap();
        let project = tempdir().unwrap();

        // Add files in reverse alphabetical order
        fs::write(temp.path().join("z.txt"), "z").unwrap();
        fs::write(temp.path().join("a.txt"), "a").unwrap();
        fs::write(temp.path().join("m.txt"), "m").unwrap();

        let summary = compare_trees(temp.path(), project.path()).unwrap();
        assert_eq!(summary.added, vec!["a.txt", "m.txt", "z.txt"]);
    }
}
