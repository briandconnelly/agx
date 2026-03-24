# Phase 4, Task 4: `agx diff` — Show Changes Since Last Generate

## Overview

Add `agx diff` command that shows what would change if `agx generate` were run now.
This is a dry-run that compares the current generated files against what would be produced from the current lockfile, without actually writing anything.

## Implementation

### 1. Create `src/cli/diff.rs`

```rust
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Only check a specific backend
    #[arg(long)]
    pub backend: Option<String>,
}
```

### 2. Core logic

The diff command:
1. Loads the manifest and lockfile (same preamble as generate.rs)
2. For each backend, generates output to a **tempdir** instead of the project dir
3. Compares the tempdir output against the actual project files
4. Prints a summary of differences

### 3. Comparison logic

For each generated file, compare against the existing file in the project:
- **New file**: file exists in tempdir but not in project → print in green: `+ <path>`
- **Modified file**: file exists in both but content differs → print in yellow: `~ <path>`
- **Unchanged file**: same content → skip (or print in dim if `--verbose`)
- **Deleted file**: file exists in project with agx marker but not in tempdir → print in red: `- <path>`

For modified files, optionally show a unified diff (line-by-line comparison).
Implement a simple diff:
- Show lines prefixed with `-` (removed) and `+` (added)
- Use `console::style` for red/green coloring
- Limit diff output to first 20 lines per file with a "... and N more lines" trailer

### 4. Context files

Also diff the context files (CLAUDE.md, GEMINI.md, AGENTS.md) by generating them to tempdir and comparing.

### 5. Exit code

- Exit 0 if no differences found
- Exit 1 if differences exist (useful for CI: `agx diff || echo "generated files are stale"`)

Use `std::process::exit(1)` after printing the summary if there are diffs.

### 6. Register the command

- Add `pub mod diff;` in `src/cli/mod.rs`
- Add `Diff(diff::DiffArgs)` variant to `Commands` enum
- Add dispatch arm

### Tests

- Test with no differences → empty output
- Test with a modified file → shows the path and diff
- Test with a new file → shows as added
- Test the simple line diff algorithm

## Important Notes
- Only modify: `src/cli/diff.rs` (new), `src/cli/mod.rs`
- No new dependencies — use tempfile (already a dependency) for the temp generation
- Reuse the generation logic from generate.rs but target a tempdir
- Keep the diff algorithm simple — no need for Myers diff, just show changed lines
