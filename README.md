# agx -- Agent Environment Manager

A package manager for AI agent environments.
Think Cargo or uv, but for managing plugins, skills, subagents, and MCP servers across agent frameworks.

## Why agx?

AI coding agents are becoming core infrastructure for development teams.
But managing their environments -- plugins, skills, MCP servers, context files, model settings -- is a mess:

- **Environment drift.** One developer has a working MCP server config; another has a stale one.
Skills get added on one machine and forgotten on another.
There is no lockfile, no pinning, no reproducibility.
- **Multi-framework duplication.** A project using Claude Code and Gemini CLI maintains two parallel sets of config files that are 80% identical.
Changes to shared instructions require editing multiple files.
- **No sharing mechanism.** Useful agent skills and plugins are copy-pasted between projects.
There is no registry, no versioning, no way to discover what others have built.
- **Manual plumbing.** Each framework has its own directory layout, settings format, and conventions.
Setting up a new developer means a wiki page and 20 minutes of hand-editing JSON.

agx eliminates all of this.
One manifest, one lockfile, one command.

## Key Features

### One manifest, many backends

Write `agent-environment.yml` once.
agx generates the correct config files for Claude Code, Gemini CLI, Codex, Cursor, and Windsurf -- simultaneously, from one source of truth.

### Reproducible environments

A lockfile (`agent-environment.lock`) pins exact versions, commit SHAs, and content hashes.
Every developer on the team gets an identical agent environment.
No more "works on my machine" for AI tooling.

### Import existing projects

Already have a `.claude/` directory, `.mcp.json`, agents, and skills?
Run `agx import` to reverse-engineer a manifest from your existing config -- instant value, no manual YAML writing.

```bash
agx import              # scan and create manifest from existing files
agx import --dry-run    # preview what would be imported
agx import --backend claude-code,gemini-cli  # import specific backends only
```

### Federated plugin registries

Share skills and agents through GitHub-hosted marketplace registries.
Use the official registry, community marketplaces, or host private registries for your organization -- no infrastructure required.

### Context file management

Shared project instructions go in `CONTEXT.md`.
Backend-specific additions go in overlays (`CONTEXT.claude-code.md`, `CONTEXT.gemini-cli.md`).
agx merges them during generation, eliminating the duplication that plagues multi-framework projects.

### Isolation modes

Control how much of the host environment leaks into your agent:
- **strict** -- only project-scoped assets; user and global config suppressed (default)
- **permissive** -- project assets layer on top of user config
- **bare** -- maximum isolation for CI/CD pipelines

## Quick Start

```bash
# Install agx
cargo install agx

# New project: interactive wizard
agx init

# Existing project: import current config
agx import

# Add a plugin from a registry
agx add plugin quality-review --registry official --version ">=1.0.0"

# Resolve, download, and generate backend configs
agx install

# Launch your agent in the managed environment
agx run
```

## Use Cases

### Teams standardizing on agent tooling

Your team uses Claude Code.
Everyone has slightly different settings, skills, and MCP servers.
Some developers have plugins others don't.
When someone joins, they spend an hour copying config files.

With agx: check in `agent-environment.yml` and `agent-environment.lock`.
New developers run `agx install` and get an identical environment in seconds.

### Multi-framework projects

Your project supports both Claude Code and Gemini CLI.
You maintain parallel config directories, duplicating shared instructions and skills across frameworks.
Every change requires editing two (or more) sets of files.

With agx: write shared context once in `CONTEXT.md`, add backend-specific overlays where needed.
Skills and agents are declared once in the manifest and generated for all active backends.

### Organizations sharing internal tools

Your company has built custom agent skills for code review, deployment, and compliance checks.
Today these are copy-pasted between repositories, with no versioning or update mechanism.

With agx: publish skills to a private GitHub-hosted registry.
Projects declare dependencies with semver constraints.
Updates propagate through `agx update`.

### CI/CD pipelines

Your CI needs to run agents in a clean, reproducible environment without developer-specific config leaking in.

With agx: use `bare` isolation mode.
The lockfile guarantees the exact same agent environment in CI as in development.
No manual setup, no environment-specific scripts.

## Manifest Example

The `agent-environment.yml` file is the single source of truth for your agent environment:

```yaml
name: my-agent-project
version: 0.1.0
description: A web application with AI-assisted development

# Target backends -- agx generates config for each
backends:
  claude-code:
    default: true
    isolation: strict
    model: sonnet
  gemini-cli:
    isolation: permissive

# Marketplace registries for plugin discovery
registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official

# Plugins from registries or git
plugins:
  quality-review:
    registry: official
    version: ">=1.0.0, <2.0.0"
  custom-plugin:
    git: https://github.com/someone/their-plugin.git
    tag: v3.0.0

# Local or remote skills
skills:
  deploy-checklist:
    path: ./skills/deploy-checklist

# Agent definitions with optional model overrides
agents:
  code-reviewer:
    path: ./agents/code-reviewer.md
    model: opus

# MCP servers (remote or local)
mcp-servers:
  playwright:
    url: https://mcp.playwright.dev/sse
  filesystem:
    command: npx
    args: ["@anthropic-ai/mcp-server-filesystem", "./src"]
    env:
      ALLOW_WRITE: "true"

# Context file management
context:
  base: ./CONTEXT.md
  overlays:
    claude-code: ./CONTEXT.claude-code.md

# Lifecycle hooks
hooks:
  post-install:
    - echo "Environment ready!"
```

## Commands

| Command | Description |
|---|---|
| `agx init` | Initialize a new agx project (interactive wizard) |
| `agx import` | Import existing agent config into an `agent-environment.yml` |
| `agx install` | Resolve, download, and generate backend configurations |
| `agx add` | Add a dependency to the manifest |
| `agx remove` | Remove a dependency from the manifest |
| `agx update` | Re-resolve dependencies within manifest constraints |
| `agx lock` | Resolve and write lockfile without downloading or generating |
| `agx generate` | Regenerate backend configs from existing lockfile and cache |
| `agx diff` | Show what would change if `agx generate` were run now |
| `agx run` | Launch a backend in the managed environment |
| `agx list` | Show the current dependency tree |
| `agx graph` | Visualize the dependency graph (tree or DOT format) |
| `agx check` | Validate manifest and detect conflicts |
| `agx info` | Show details about an installed asset |
| `agx which` | Show the resolved source and file path of an asset |
| `agx outdated` | List assets with newer versions available |
| `agx clean` | Remove `.agx/` and all generated backend directories |
| `agx cache` | Cache management commands |
| `agx registry` | Registry management commands |
| `agx publish` | Publish an asset to a marketplace registry |
| `agx backends` | List available backends |
| `agx completions` | Generate shell completions |

## Backends

agx supports multiple agent frameworks through a pluggable backend system.
Each backend generates the correct directory layout and config files for its framework.

| Backend | Context File | Generated Output |
|---|---|---|
| `claude-code` | `CLAUDE.md` | `.claude/settings.json`, `.claude/agents/`, `.claude/skills/`, `.mcp.json` |
| `gemini-cli` | `GEMINI.md` | `.gemini/settings.json`, `.gemini/skills/` |
| `codex` | `AGENTS.md` | `.codex/agents/`, `.codex/skills/` |
| `cursor` | `AGENTS.md` | `.cursor/agents/`, `.cursor/skills/` |
| `windsurf` | `AGENTS.md` | `.agx/agents/`, `.agx/skills/` |
| `generic` | `AGENTS.md` | `.agx/agents/`, `.agx/skills/` |

A single manifest can target multiple backends simultaneously.
Run `agx backends` to list all available backends.

## How It Works

agx uses a three-phase pipeline, similar to how Cargo or uv operate:

1. **Resolve** -- The manifest (`agent-environment.yml`) is parsed and dependency versions are solved.
Git refs are resolved to pinned commit SHAs.
The result is written to `agent-environment.lock`, which should be committed to version control for reproducibility.

2. **Fetch** -- Resolved assets are downloaded to a global cache (`$XDG_CACHE_HOME/agx`).
The cache is content-addressed and shared across all projects, so identical dependencies are only downloaded once.

3. **Generate** -- Cached assets are rendered into backend-specific directories.
Context files are assembled by merging the base `CONTEXT.md` with backend-specific overlays.
Agent frontmatter is injected with model overrides.
MCP server configs are written in the backend's expected format.

Running `agx install` performs all three phases.
You can also run them individually with `agx lock`, `agx cache`, and `agx generate`.

## Configuration

### Environment Variables

| Variable | Description |
|---|---|
| `AGX_OFFLINE=1` | Disable all network access (use only cached data) |
| `AGX_CACHE_DIR` | Override the default global cache directory |
| `AGX_LOG=debug` | Enable verbose logging (`trace`, `debug`, `info`, `warn`, `error`) |
| `NO_COLOR=1` | Disable colored terminal output |

### File Layout

```
my-project/
  agent-environment.yml       # Manifest (human-edited, committed)
  agent-environment.lock      # Lockfile (auto-generated, committed)
  CONTEXT.md                  # Shared base context (committed)
  CONTEXT.claude-code.md      # Backend-specific overlay (committed)
  agents/                     # Local agent definitions (committed)
  skills/                     # Local skill definitions (committed)
  .agx/                       # Workspace metadata (gitignored)
  CLAUDE.md                   # Generated context (gitignored)
  .claude/                    # Generated backend dir (gitignored)
  .mcp.json                   # Generated MCP config (gitignored)
```

Only the manifest, lockfile, context source files, and local assets are committed to version control.
Everything in `.agx/` and all generated backend files are reproducible from these inputs.

## License

MIT
