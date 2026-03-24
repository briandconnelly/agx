# Phase 5, Task 4: README + Example Projects

## Overview

Create a polished README.md and example project directories that demonstrate real usage.

## 1. README.md

Create a comprehensive README.md at the project root.
Content:

### Structure

```markdown
# agx — Agent Environment Manager

One-line description: A package manager for AI agent environments.

## What is agx?

2-3 paragraphs explaining the problem (managing configs for AI coding agents is tedious and non-reproducible) and the solution (agx manages plugins, skills, subagents, and MCP servers with a single manifest, lockfile, and generate pipeline — like Cargo for agent environments).

## Quick Start

Step-by-step:
1. Install: `cargo install agx` (or curl install script)
2. Initialize: `agx init`
3. Add a plugin: `agx add plugin quality-review --registry official --version ">=1.0.0"`
4. Install: `agx install`
5. Run: `agx run`

## Manifest Example

Show a realistic `agent-environment.yml` with annotations.

## Commands

Table of all commands with one-line descriptions:
| Command | Description |
|---|---|
| `agx init` | Initialize a new project or import existing |
| `agx install` | Resolve, download, and generate |
| ... | ... |

## Backends

List supported backends (claude-code, gemini-cli, codex, cursor, windsurf, generic) with what each generates.

## How It Works

Brief explanation of the three-phase pipeline:
1. **Resolve** — manifest → lockfile (version solving, git ref resolution)
2. **Fetch** — download to global cache (XDG-compliant, content-addressed)
3. **Generate** — render cached assets into backend-specific files

## Configuration

Key configuration options:
- `AGX_OFFLINE=1` — disable network access
- `NO_COLOR=1` — disable colored output
- `AGX_LOG=debug` — verbose logging

## License

MIT
```

## 2. Example: `examples/claude-code-project/`

A minimal but complete Claude Code project:

- `agent-environment.yml`:
  ```yaml
  name: my-claude-project
  backends:
    claude-code:
      isolation: strict
      model: sonnet
  skills:
    deploy-checklist:
      path: ./skills/deploy-checklist
  agents:
    code-reviewer:
      path: ./agents/code-reviewer.md
      model: opus
  mcp-servers:
    filesystem:
      command: npx
      args: ["@anthropic-ai/mcp-server-filesystem", "./src"]
  context:
    base: ./CONTEXT.md
  hooks:
    post-install:
      - echo "Installation complete!"
  ```
- `skills/deploy-checklist/SKILL.md` — example skill content
- `agents/code-reviewer.md` — example agent with frontmatter
- `CONTEXT.md` — example project context

## 3. Example: `examples/multi-backend-project/`

A project using Claude Code + Gemini CLI + generic:

- `agent-environment.yml` with three backends, shared context with overlays
- `CONTEXT.md` — shared base context
- `CONTEXT.claude-code.md` — Claude-specific overlay
- `CONTEXT.gemini-cli.md` — Gemini-specific overlay
- `skills/shared-skill/SKILL.md` — a skill used by all backends

## Important Notes
- Only create new files: `README.md`, `examples/` directory tree
- Do NOT modify any Rust source files, Cargo.toml, or existing docs
- Do NOT create any .md files in the root other than README.md
- Keep examples realistic but simple — they should work with `agx install` if dependencies were available
