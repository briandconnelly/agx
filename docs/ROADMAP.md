# agx — Implementation Roadmap & Deliverables

**Target audience:** Solo developer building with Claude Code agent swarms (high parallelization capacity).

---

## Artifact List

Everything that needs to exist for agx to be a real, shippable product.

### Documents

| Artifact | Purpose | When |
|---|---|---|
| `agx-spec.md` | Design specification (✅ done) | Phase 0 |
| `agent-environment.schema.json` | JSON Schema for manifest validation + editor autocomplete | Phase 1 |
| `agent-environment.lock.schema.json` | JSON Schema for lockfile format | Phase 1 |
| `README.md` | Project overview, install instructions, quickstart | Phase 1 |
| `CONTRIBUTING.md` | How to add backends, report bugs, submit PRs | Phase 3 |
| `LICENSE` | Open-source license (recommend MIT or Apache 2.0) | Phase 1 |
| Documentation site | Full user docs (mdbook, Docusaurus, or similar) | Phase 3 |
| ADRs (Architecture Decision Records) | Key decisions log (why Rust, why YAML, etc.) | Ongoing |

### Software — Core CLI

| Component | Description | Phase |
|---|---|---|
| **Manifest parser** | Read/write `agent-environment.yml`, validate against schema | Phase 1 |
| **Lockfile manager** | Read/write/diff `agent-environment.lock` | Phase 1 |
| **Registry client** | Clone/fetch git-based marketplace indexes, cache metadata | Phase 1 |
| **Git source client** | Clone repos, resolve tags/branches/commits to SHAs, sparse checkout | Phase 1 |
| **Version resolver** | Semver constraint solver for plugin versions | Phase 1 |
| **Integrity manager** | SHA-256 hashing, verification on install | Phase 1 |
| **Global cache manager** | XDG-compliant cache dir, content-addressed storage, clean/list | Phase 1 |
| **Generator engine** | Orchestrates per-backend generation from lockfile + cache | Phase 1 |
| **Context merger** | Reads CONTEXT.md + overlays, merges, writes backend-specific files | Phase 2 |
| **Init scaffolder** | New project wizard, existing project scanner | Phase 2 |
| **CLI framework** | Argument parsing, subcommands, colored output, interactive prompts | Phase 1 |

### Software — Backend Modules

| Backend | Generates | Phase |
|---|---|---|
| `claude-code` | `.claude/`, `.mcp.json`, `CLAUDE.md`, settings, agents, skills | Phase 1 |
| `gemini-cli` | `.gemini/`, `GEMINI.md`, settings, agents, skills | Phase 2 |
| `generic` (AGENTS.md) | `AGENTS.md`, skill directories | Phase 2 |
| `codex` | `.codex/`, skills | Phase 3 |
| `cursor` | `.cursor/`, rules | Phase 3 |

### Software — Commands

| Command | Phase |
|---|---|
| `agx init` (new project, basic) | Phase 1 |
| `agx install` | Phase 1 |
| `agx generate` | Phase 1 |
| `agx lock` | Phase 1 |
| `agx add` / `agx remove` | Phase 1 |
| `agx list` | Phase 1 |
| `agx check` | Phase 1 |
| `agx clean` | Phase 1 |
| `agx cache dir` / `clean` / `list` | Phase 1 |
| `agx run` | Phase 2 |
| `agx update` | Phase 2 |
| `agx init` (existing project scan) | Phase 2 |
| `agx info` / `agx which` / `agx outdated` | Phase 2 |
| `agx registry add` / `remove` / `list` / `refresh` | Phase 2 |
| `agx init --scan-only` | Phase 2 |

### Testing & Validation

| Artifact | Description | Phase |
|---|---|---|
| Unit test suite | Parser, resolver, cache manager, generator, context merger | Phase 1+ |
| Integration test suite | End-to-end `init → install → generate → run` workflows | Phase 2 |
| Example project: `agx-example-claude` | Minimal Claude Code project managed by agx | Phase 1 |
| Example project: `agx-example-multi` | Multi-backend project (Claude + Gemini) | Phase 2 |
| Example project: `agx-example-monorepo` | Monorepo with child context files | Phase 3 |
| CI pipeline | GitHub Actions: build, test, release binaries | Phase 1 |

### Distribution

| Artifact | Description | Phase |
|---|---|---|
| Pre-built binaries | Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), Windows | Phase 1 |
| Install script | `curl -sSf https://agx.dev/install.sh \| sh` (à la Rust/uv) | Phase 2 |
| Homebrew formula | `brew install agx` | Phase 2 |
| npm wrapper | `npx agx` (downloads binary, like `@anthropic-ai/claude-code`) | Phase 3 |
| Cargo install | `cargo install agx` (if built in Rust) | Phase 1 (free) |

---

## Implementation Roadmap

**Capacity model:** With Claude Code agent swarms, you can run 4–8 agents in parallel on independent components. The phases below are compressed accordingly, with parallelization maps showing which tasks can run simultaneously. Total estimated timeline: **4–5 weeks to v0.1.0 stable** (vs. 12 weeks for a solo human developer).

### Phase 0: Foundation (Day 1–2)
**Goal: Project skeleton, build system, CI.**

- [ ] Choose language: **Rust recommended** (fast, single binary, Cargo ecosystem is a natural fit; alternatives: Go, TypeScript)
- [ ] Initialize repo with `cargo init`
- [ ] Set up CLI framework (`clap` for arg parsing)
- [ ] Set up CI (GitHub Actions: build + test on Linux/macOS/Windows)
- [ ] Create `README.md` and `LICENSE`
- [ ] Create `agent-environment.schema.json` from the spec (enables editor autocomplete from day one)
- [ ] Add spec as `docs/SPEC.md` in the repo

**Deliverable:** A binary that prints `agx 0.1.0-dev` and has stubbed subcommands.

---

### Phase 1: MVP — Claude Code Backend (Days 3–10)
**Goal: A working `agx init → install → generate` pipeline for Claude Code projects.**

This is the critical path. Everything else builds on this.

**Parallelization map — spin up 5 agents simultaneously:**

```
Agent 1: Manifest parser + lockfile manager
  ├── YAML manifest parser (serde + serde_yaml)
  ├── Manifest validation against schema
  └── Lockfile data model and serializer

Agent 2: Git + registry client
  ├── Git clone/fetch operations (git2 crate or shell to git)
  ├── Sparse checkout for individual skills/agents
  ├── Registry index fetcher (marketplace.json parser)
  └── Tag/branch/commit → SHA resolution

Agent 3: Version resolver + integrity
  ├── Semver constraint parser (semver crate)
  ├── Version resolution within constraints
  ├── Conflict detection (same-named assets from different plugins)
  └── SHA-256 integrity hashing + verification

Agent 4: Cache manager + CLI commands
  ├── XDG-compliant global cache directory
  ├── Content-addressed storage layout
  ├── agx cache dir / clean / list
  ├── agx list, agx check, agx clean
  └── .gitignore generation

Agent 5: Claude Code backend generator
  ├── Generate .claude/settings.json (isolation, model, permissions)
  ├── Copy/place agents into .claude/agents/ (with model/params injection)
  ├── Copy/place skills into .claude/skills/
  ├── Generate .mcp.json from manifest declarations
  ├── Merge settings section into generated config
  └── Auto-generated file headers
```

**Integration (after agents converge):**

- [ ] Wire up `agx lock` — resolver + git client + lockfile writer
- [ ] Wire up `agx install` — lock + fetch to cache + generator
- [ ] Wire up `agx generate` — generator from existing lockfile + cache
- [ ] Wire up `agx init` — basic new-project scaffolder
- [ ] Wire up `agx add` / `agx remove` — manifest editing

**Deliverable:** A user can `agx init`, add plugins/skills/agents/MCP servers, run `agx install`, and get a working `.claude/` + `.mcp.json`. Teammates can clone + `agx install` to reproduce.

**Milestone: v0.1.0-alpha release.**

---

### Phase 2: Full Feature Set (Days 11–18)
**Goal: Multi-backend, `agx run`, existing project import, context files.**

**Parallelization map — spin up 5 agents simultaneously:**

```
Agent 1: Context file system
  ├── CONTEXT.md + overlay merger
  ├── Backend-specific output filenames (CLAUDE.md, GEMINI.md, AGENTS.md)
  ├── Child context files (monorepo subdirectories)
  └── Auto-generated marker comments

Agent 2: agx run + update
  ├── Pre-flight checks + staleness detection
  ├── Backend launch command abstraction
  ├── --model / --effort runtime overrides
  ├── Exit code forwarding
  ├── agx update / agx update <n>
  └── agx outdated

Agent 3: Existing project scanner
  ├── File detection engine (.claude/, .mcp.json, CLAUDE.md, .gemini/, etc.)
  ├── Plugin source matching against registries
  ├── Context file splitter (CLAUDE.md → CONTEXT.md + CONTEXT.claude-code.md)
  ├── agx init existing project flow
  └── agx init --scan-only / --force

Agent 4: Gemini CLI backend
  ├── Generate .gemini/settings.json
  ├── Place agents into .gemini/agents/ (with model/params in frontmatter)
  ├── Place skills into .gemini/skills/
  ├── Map parameters to modelConfigs system
  └── Generate GEMINI.md from context merger

Agent 5: Inspection + registry commands
  ├── agx info <asset>
  ├── agx which <asset>
  ├── agx backends
  ├── agx registry add/remove/list/refresh
  └── Private registry auth (env var tokens)
```

**Deliverable:** Full multi-backend support, seamless adoption for existing projects, `agx run` as the primary launch command.

**Milestone: v0.1.0-beta release.**

---

### Phase 3: Polish + Ecosystem (Days 19–25)
**Goal: Production-ready release, documentation, distribution.**

**Parallelization map — spin up 4 agents simultaneously:**

```
Agent 1: Documentation site
  ├── User guide (quickstart, manifest reference, CLI reference)
  ├── Backend guide (Claude Code, Gemini CLI, generic)
  ├── Migration guide (adopting agx in existing projects)
  └── CONTRIBUTING.md

Agent 2: Distribution
  ├── Cross-compiled binaries (linux x86_64/aarch64, macOS Intel/ARM, Windows)
  ├── Install script (curl | sh)
  ├── Homebrew formula
  └── Shell completions (bash, zsh, fish, PowerShell)

Agent 3: Example projects + integration tests
  ├── agx-example-claude (minimal Claude Code project)
  ├── agx-example-multi (Claude + Gemini)
  ├── agx-example-monorepo (child context files)
  ├── End-to-end integration test suite
  └── CI: build + test + release pipeline

Agent 4: Generic backend + hardening
  ├── Generic AGENTS.md backend module (Codex, Cursor, Windsurf)
  ├── Error message audit (clear, actionable, with suggestions)
  ├── Performance audit (large registries, many plugins)
  └── Edge case hardening (offline mode, corrupt cache, etc.)
```

**Milestone: v0.1.0 stable release.**

---

### Phase 4+: Future (Post-launch)

Items from the spec's "Future Considerations" section, prioritized by user demand:

| Feature | Description | Effort |
|---|---|---|
| `agx publish` | Publish skills/plugins to a marketplace | Medium |
| Workspace support | Monorepo with multiple `agent-environment.yml` files | Large |
| Backend plugins | Third-party backends as installable extensions | Large |
| `agx graph` | Visualize dependency tree | Small |
| Hook support | Pre/post install/generate hooks in manifest | Medium |
| Registry integrity pinning | Pin the marketplace index to a commit SHA | Small |
| Codex/Cursor/Windsurf backends | Additional built-in backends | Medium each |

## Technology Recommendations

### Language: Rust

**Why Rust over alternatives:**

| Consideration | Rust | Go | TypeScript/Node |
|---|---|---|---|
| Single binary, no runtime | ✅ | ✅ | ❌ (needs Node) |
| Startup speed | ~1ms | ~5ms | ~100ms+ |
| Ecosystem precedent | Cargo, uv, ripgrep | Docker, Terraform | npm, Bun |
| YAML/TOML parsing | `serde` (excellent) | Adequate | Adequate |
| Git operations | `git2` crate (libgit2) | `go-git` | `simple-git` (shells out) |
| Semver resolution | `semver` crate | `go-semver` | `node-semver` |
| Cross-compilation | Good (cross-rs) | Excellent | N/A |
| Learning curve | Steep | Moderate | Low |

Rust is the strongest choice if you're comfortable with it. It produces the kind of fast, single-binary CLI that users expect from modern dev tools (this is what uv, Cargo, and ripgrep all use). If Rust is too steep, Go is a solid second choice.

### Key Crates (Rust)

| Crate | Purpose |
|---|---|
| `clap` | CLI argument parsing with derive macros |
| `serde` + `serde_yaml` | Manifest/lockfile serialization |
| `semver` | Version constraint parsing and resolution |
| `git2` | Git operations (clone, fetch, checkout) |
| `sha2` | SHA-256 integrity hashing |
| `dirs` | XDG-compliant directory paths |
| `dialoguer` | Interactive prompts (init wizard) |
| `indicatif` | Progress bars (downloads) |
| `console` | Colored terminal output |
| `toml` | If you ever need TOML support |
| `jsonschema` | Manifest validation against JSON Schema |
| `tempfile` | Temporary directories for atomic operations |
| `walkdir` | Filesystem scanning (existing project detection) |
| `similar` | Text diffing (context file splitting heuristics) |

---

## What to Build First (Critical Path)

If you have limited time and want the fastest path to something usable:

```
1. Manifest parser + lockfile writer        (the data model)
2. Git client + cache manager               (downloading things)
3. Claude Code generator                    (placing things)
4. agx install + agx generate               (the core loop)
5. agx init (new project only)              (onboarding)
```

That's your v0.0.1. Everything else layers on top. The core insight is that `agx` is fundamentally just three operations: **resolve** (figure out what versions to use), **fetch** (download them to cache), and **generate** (place them where the backend expects). Get those three right and the rest is UX.
