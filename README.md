# agx -- Agent Environment Manager

A package manager for AI agent environments. Think Cargo or uv, but for managing plugins, skills, subagents, and MCP servers across agent frameworks.

## What is agx?

Managing configurations for AI coding agents is tedious and non-reproducible. Each framework -- Claude Code, Gemini CLI, Codex, Cursor -- has its own config format, directory layout, and conventions. When a team works on a project, there is no reliable way to ensure every developer's agent environment matches: skills drift, MCP servers get misconfigured, and context files go stale. It is the "it works on my machine" problem, but for AI tooling.

agx solves this with a single `agent-environment.yml` manifest that declares everything your agents need: plugins, skills, subagents, MCP servers, context files, and model settings. A lockfile pins exact versions for reproducibility. A global cache avoids redundant downloads. And a generate step renders the right files for whichever backend you use -- all from one source of truth.

The workflow mirrors established package managers. You write a manifest, run `agx install`, and get a fully configured agent environment. Check the manifest and lockfile into version control; everything else is regenerated on demand.

## Quick Start

```bash
# Install agx
cargo install agx

# Initialize a new project
agx init

# Add a plugin from a registry
agx add plugin quality-review --registry official --version ">=1.0.0"

# Resolve, download, and generate backend configs
agx install

# Launch your agent in the managed environment
agx run
```

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
| `agx init` | Initialize a new agx project or import existing configuration |
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
| `agx backends` | List available backends |
| `agx completions` | Generate shell completions |

## Backends

agx supports multiple agent frameworks through a pluggable backend system. Each backend knows how to generate the correct directory layout and config files for its framework.

| Backend | Context File | Generated Output |
|---|---|---|
| `claude-code` | `CLAUDE.md` | `.claude/settings.json`, `.claude/agents/`, `.claude/skills/`, `.mcp.json` |
| `gemini-cli` | `GEMINI.md` | `.gemini/settings.json`, `.gemini/skills/` |
| `codex` | `AGENTS.md` | `.codex/agents/`, `.codex/skills/` |
| `cursor` | `AGENTS.md` | `.cursor/agents/`, `.cursor/skills/` |
| `generic` | `AGENTS.md` | `.agx/agents/`, `.agx/skills/` |

A single manifest can target multiple backends simultaneously. Run `agx backends` to list all available backends.

## How It Works

agx uses a three-phase pipeline, similar to how Cargo or uv operate:

1. **Resolve** -- The manifest (`agent-environment.yml`) is parsed and dependency versions are solved. Git refs are resolved to pinned commit SHAs. The result is written to `agent-environment.lock`, which should be committed to version control for reproducibility.

2. **Fetch** -- Resolved assets are downloaded to a global cache (`$XDG_CACHE_HOME/agx`). The cache is content-addressed and shared across all projects, so identical dependencies are only downloaded once.

3. **Generate** -- Cached assets are rendered into backend-specific directories. Context files are assembled by merging the base `CONTEXT.md` with backend-specific overlays. Agent frontmatter is injected with model overrides. MCP server configs are written in the backend's expected format.

Running `agx install` performs all three phases. You can also run them individually with `agx lock`, `agx cache`, and `agx generate`.

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

Only the manifest, lockfile, context source files, and local assets are committed to version control. Everything in `.agx/` and all generated backend files are reproducible from these inputs.

## License

MIT
