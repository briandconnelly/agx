# agx — Agent Environment Manager

A package manager for AI agent environments. Think Cargo or uv, but for managing plugins, skills, subagents, and MCP servers across agent frameworks (Claude Code, Gemini CLI, Codex, Cursor, etc.).

## Core Operations

agx has three fundamental operations — everything else is UX on top:

1. **Resolve** — figure out what versions/commits to use (manifest → lockfile)
2. **Fetch** — download resolved assets to a global cache
3. **Generate** — place cached assets into backend-specific directories

## Build Commands

```bash
cargo build          # debug build
cargo build -r       # release build
cargo test           # run all tests
cargo run -- --help  # run with args
cargo run -- init    # example: run init command
```

## Module Map

| Module | Responsibility |
|---|---|
| `src/cli/` | CLI argument parsing (clap derive) and command dispatch. One file per command group. |
| `src/manifest/` | Parse/write `agent-environment.yml`. `types.rs` has all manifest data types — this is the shared contract. `validate.rs` checks structural correctness. |
| `src/lockfile/` | Parse/write `agent-environment.lock`. `types.rs` has lockfile data types. |
| `src/resolver/` | Semver constraint solving, version resolution, conflict detection. |
| `src/sources/` | Fetching assets: `git.rs` (shells out to git), `registry.rs` (marketplace index), `local.rs` (local paths). |
| `src/cache/` | Global cache management (XDG-compliant). `integrity.rs` for SHA-256 hashing/verification. |
| `src/backends/` | Per-backend code generation. `mod.rs` defines the `Backend` trait. `claude_code.rs`, `gemini_cli.rs`, `generic.rs` are implementations. |
| `src/context/` | CONTEXT.md + overlay merging into backend-specific files (CLAUDE.md, GEMINI.md, etc.). |
| `src/config.rs` | Global config: env vars, file paths, defaults. |

## Coding Conventions

- **Error handling:** Use `thiserror` for typed errors in library code (manifest, lockfile, resolver, sources, cache, backends). Use `anyhow::Result` in CLI handlers (`src/cli/`).
- **Doc comments:** All public types and functions need `///` doc comments.
- **Tests:** Unit tests go in `#[cfg(test)] mod tests` in the same file. Integration tests go in `tests/`.
- **Serde:** All data types that touch YAML/JSON derive `Serialize, Deserialize, Debug, Clone`.
- **Collections:** Use `BTreeMap` over `HashMap` for deterministic serialization order.
- **Git operations:** Shell out to `git` binary (no libgit2/git2 crate). Keep it simple for now.
- **Dependencies:** Don't add new crates without a strong reason. The current set covers the needs.

## Key Files

- `docs/SPEC.md` — full design specification (the source of truth for behavior)
- `docs/ROADMAP.md` — implementation roadmap with phase breakdown
- `agent-environment.schema.json` — JSON Schema for manifest validation + editor autocomplete
- `src/manifest/types.rs` — **the** shared contract; all modules depend on these types

## Current Status

Phase 0 complete. CLI skeleton with all commands stubbed. Manifest and lockfile types defined and tested. Backend trait defined. Ready for Phase 1 implementation.
