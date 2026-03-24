# Phase 5, Task 2: Windsurf Backend

## Overview

Add a Windsurf backend following the same pattern as Cursor/Codex (AGENTS.md-based generic backends).

## What Windsurf Generates

Windsurf (by Codeium) uses:
- `.windsurf/` directory for configuration
- `.windsurf/rules/` for custom rules/skills
- `AGENTS.md` for project context (same standard as Cursor, Codex)

## Implementation

### 1. Create `src/backends/windsurf.rs`

Follow the exact pattern of `cursor.rs`:
- `WindsurfBackend` struct implementing `Backend`
- `name()` → `"windsurf"`
- `context_filename()` → `"AGENTS.md"`
- `generate()`:
  1. Generate `AGENTS.md` from base context
  2. Copy skills to `.windsurf/rules/`
- `run_command()` → bail: "windsurf backend is an IDE — open the project directory in Windsurf instead"

### 2. Register

- Add `pub mod windsurf;` in `src/backends/mod.rs`
- Register in `get_backend()`:
  ```rust
  "windsurf" => Some(Box::new(windsurf::WindsurfBackend)),
  ```

### 3. Update backends_cmd.rs

Add "windsurf" to `BUILTIN_BACKENDS` with description "Windsurf (Codeium)".
Update the test assertion for the backend count.

### 4. Update init.rs

Add "windsurf" to the backend selection list in the init wizard (where claude-code, gemini-cli, codex, cursor are listed).

### Tests

Follow the pattern in `cursor.rs`:
- Generate with context → verify AGENTS.md
- Generate without context → verify placeholder
- Skill copy to .windsurf/rules/ → verify files with markers
- Run command fails with helpful message
- Idempotent generation
- At least 6 tests

## Important Notes
- Only modify: `src/backends/windsurf.rs` (new), `src/backends/mod.rs`, `src/cli/backends_cmd.rs`, `src/cli/init.rs`
- Do NOT modify Cargo.toml, src/cli/mod.rs, or src/lib.rs
