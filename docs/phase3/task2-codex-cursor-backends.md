# Phase 3, Task 2: Codex + Cursor Backends

Implement two new backend modules following the existing pattern in `src/backends/`.

## Codex Backend (`src/backends/codex.rs`)

### What it generates
- `.codex/` directory
- `.codex/skills/` — skill SKILL.md files (same pattern as generic backend)
- `AGENTS.md` — from base context (same as generic, since Codex supports AGENTS.md)

### `run_command`
```
codex --skills-dir .codex/skills/ --model <model>
```

### Models supported
`o3`, `o4-mini`, `codex-mini`

### Parameters
`temperature`, `max_tokens`, `reasoning_effort` — map to command-line flags or config as appropriate.

### Implementation
- Create `src/backends/codex.rs` implementing `Backend` trait
- `name()` returns `"codex"`
- `context_filename()` returns `"AGENTS.md"`
- `generate()`:
  1. Create `.codex/` directory
  2. Copy skills to `.codex/skills/` (reuse `copy_dir_recursive` from `super`)
  3. Generate `AGENTS.md` from base context (can share logic with generic backend)
- `run_command()`: builds `codex` command with `--skills-dir` and model flags

## Cursor Backend (`src/backends/cursor.rs`)

### What it generates
- `.cursor/` directory
- `.cursor/rules/` — skill/context rules
- `AGENTS.md` — from base context

### `run_command`
Cursor is an IDE, not a CLI.
`run_command` should bail with a helpful message: "cursor backend is an IDE — open the project directory in Cursor instead".

### Implementation
- Create `src/backends/cursor.rs` implementing `Backend` trait
- `name()` returns `"cursor"`
- `context_filename()` returns `"AGENTS.md"`
- `generate()`:
  1. Create `.cursor/` directory
  2. Create `.cursor/rules/` with skill content as rule files
  3. Generate `AGENTS.md` from base context
- `run_command()`: bail with helpful message

## Registration

1. Add `pub mod codex;` and `pub mod cursor;` in `src/backends/mod.rs`
2. Register both in `get_backend()`:
   ```rust
   "codex" => Some(Box::new(codex::CodexBackend)),
   "cursor" => Some(Box::new(cursor::CursorBackend)),
   ```

## Tests

Each backend needs tests following the pattern in `generic.rs`:
- Generate with context → verify output files
- Generate without context → verify placeholder
- Skill copy → verify files present with markers
- Run command behavior (codex builds command, cursor bails)
- Idempotent generation

## Update backends_cmd.rs

Add "codex" and "cursor" to the BUILT_IN_BACKENDS list in `src/cli/backends_cmd.rs`.
