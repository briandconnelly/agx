# agx — Claude Code Kickoff Prompt

## How to use

Copy the prompt below and paste it into Claude Code from the directory containing `agx-spec.md` and `agx-roadmap.md`. This will execute Phase 0 (project foundation) and set up the scaffolding for Phase 1 parallel agents.

---

## Prompt

```
Read @agx-spec.md and @agx-roadmap.md thoroughly. These are the design specification and implementation roadmap for `agx`, a package manager for AI agent environments — think Cargo or uv, but for managing skills, plugins, subagents, and MCP servers across agent frameworks like Claude Code and Gemini CLI.

You are the lead engineer bootstrapping this project. Execute Phase 0 from the roadmap, then prepare the scaffolding for Phase 1.

## Phase 0: Do this now

1. Initialize a Rust project with `cargo init --name agx`
2. Set up the following dependencies in Cargo.toml:
   - clap (with derive feature) — CLI framework
   - serde + serde_yaml — manifest/lockfile serialization
   - serde_json — JSON schema, settings generation
   - semver — version constraint parsing
   - sha2 — integrity hashing
   - dirs — XDG-compliant paths
   - dialoguer — interactive prompts
   - indicatif — progress bars
   - console — colored output
   - tempfile — atomic file operations
   - walkdir — filesystem scanning
   - thiserror + anyhow — error handling
3. Set up the CLI skeleton using clap derive macros with all commands from the spec:
   - agx init, install, generate, lock, add, remove, update, list, check, clean
   - agx run [backend] [-- args]
   - agx cache (dir, clean, list)
   - agx registry (add, remove, list, refresh)
   - agx info, which, outdated, backends
   Each command should be a stubbed function that prints "not yet implemented" for now.
4. Create the module structure. The codebase should be organized as:
   ```
   src/
   ├── main.rs              # CLI entry point (clap)
   ├── cli/                  # CLI command handlers
   │   ├── mod.rs
   │   ├── init.rs
   │   ├── install.rs
   │   ├── generate.rs
   │   ├── add.rs
   │   ├── remove.rs
   │   ├── run.rs
   │   └── ...              # one file per command group
   ├── manifest/             # agent-environment.yml parsing + validation
   │   ├── mod.rs
   │   ├── types.rs          # all manifest data types (serde structs)
   │   └── validate.rs
   ├── lockfile/             # agent-environment.lock reading + writing
   │   ├── mod.rs
   │   └── types.rs
   ├── resolver/             # version constraint solving + conflict detection
   │   ├── mod.rs
   │   └── semver.rs
   ├── sources/              # fetching assets from various sources
   │   ├── mod.rs
   │   ├── git.rs            # git clone, fetch, sparse checkout
   │   ├── registry.rs       # marketplace index client
   │   └── local.rs          # local path resolution
   ├── cache/                # global cache management
   │   ├── mod.rs
   │   └── integrity.rs      # SHA-256 verification
   ├── backends/             # per-backend code generation
   │   ├── mod.rs            # backend trait definition
   │   ├── claude_code.rs
   │   ├── gemini_cli.rs
   │   └── generic.rs        # AGENTS.md
   ├── context/              # CONTEXT.md merging
   │   └── mod.rs
   └── config.rs             # global config (env vars, paths)
   ```
5. Define the core manifest types in `src/manifest/types.rs`. These must match the spec exactly:
   - AgentEnvironment (top-level)
   - BackendConfig (with model as enum: simple string or ModelConfig object)
   - ModelConfig (id, effort, plan_model, extended_context, available, parameters)
   - RegistryConfig
   - PluginDependency (registry source vs git source)
   - SkillDependency
   - AgentDependency (with optional model + parameters overrides)
   - McpServerConfig (url-based vs command-based)
   - SettingsConfig
   - PathsConfig
   - ContextConfig (base, overlays, children)
   Make sure all types derive Serialize, Deserialize, Debug, Clone.
6. Define the lockfile types in `src/lockfile/types.rs`:
   - Lockfile (top-level with metadata + resolved entries)
   - LockedPlugin, LockedSkill, LockedAgent (each with source, git_commit, integrity)
7. Define the Backend trait in `src/backends/mod.rs`:
   ```rust
   pub trait Backend {
       fn name(&self) -> &str;
       fn generate(&self, resolved: &ResolvedEnvironment, output_dir: &Path) -> Result<()>;
       fn context_filename(&self) -> &str;  // "CLAUDE.md", "GEMINI.md", etc.
       fn run_command(&self, config: &BackendConfig) -> Result<Command>;
   }
   ```
8. Create a minimal `agent-environment.schema.json` for the manifest. It doesn't have to be comprehensive yet — just the top-level structure with required fields.
9. Copy `agx-spec.md` and `agx-roadmap.md` to `docs/`.
10. Create a `.gitignore` for the agx project itself (Rust standard: /target, etc.).
11. Verify everything compiles with `cargo build` and that `cargo run -- --help` shows the full command tree.

## Phase 1 Preparation: Set up for parallel agents

After Phase 0 compiles, create a `CLAUDE.md` for the agx project itself with:
- Project overview (what agx is, the three core operations: resolve, fetch, generate)
- Build commands (cargo build, cargo test, cargo run)
- Module map (which modules are responsible for what)
- Coding conventions:
  - Use thiserror for library errors, anyhow in CLI handlers
  - All public types need doc comments
  - Write unit tests in the same file (mod tests)
  - Integration tests go in tests/
- A note that the spec is at docs/agx-spec.md and the roadmap is at docs/agx-roadmap.md

Then create 5 GitHub issues (or TODO files in docs/phase1/) — one per Phase 1 parallel agent from the roadmap — each containing:
- The specific deliverables for that agent
- Which modules it should work in
- The interfaces it must expose (function signatures, trait impls)
- How to test it in isolation before integration

These issues will be the prompts for the next wave of parallel Claude Code agents.

## Important constraints

- The manifest types are the shared contract between all modules. Get them right — everything else depends on them.
- Don't add git2 to dependencies yet. For the MVP, shelling out to `git` is simpler and avoids libgit2 build complexity. We can optimize later.
- Use `anyhow::Result` in CLI handlers and `thiserror` for typed errors in library code.
- Target Rust edition 2024.
```

---

## What happens next

After this prompt completes, you'll have:
1. A compiling Rust project with the full CLI skeleton
2. All manifest/lockfile types matching the spec
3. A module structure ready for parallel development
4. 5 task descriptions you can feed to parallel Claude Code agents

To kick off Phase 1, open 5 Claude Code sessions (or use agent teams/worktrees) and give each one the corresponding task file plus the project's CLAUDE.md. Each agent works on its module independently, then you integrate.
