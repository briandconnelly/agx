# Phase 4, Task 2: Hook Support (Pre/Post Install/Generate)

## Overview

Add support for declaring lifecycle hooks in the manifest that run shell commands before/after install and generate operations.

## Manifest Schema

Add a `hooks` section to `agent-environment.yml`:

```yaml
hooks:
  pre-install:
    - echo "Installing dependencies..."
  post-install:
    - ./scripts/verify-install.sh
  pre-generate:
    - echo "Generating backend configs..."
  post-generate:
    - ./scripts/post-generate.sh
    - echo "Done!"
```

## Implementation

### 1. Add types to `src/manifest/types.rs`

```rust
/// Lifecycle hooks configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default, rename = "pre-install")]
    pub pre_install: Vec<String>,
    #[serde(default, rename = "post-install")]
    pub post_install: Vec<String>,
    #[serde(default, rename = "pre-generate")]
    pub pre_generate: Vec<String>,
    #[serde(default, rename = "post-generate")]
    pub post_generate: Vec<String>,
}
```

Add to `AgentEnvironment`:
```rust
#[serde(default)]
pub hooks: Option<HooksConfig>,
```

### 2. Create `src/hooks.rs` (library module)

```rust
/// Run a list of hook commands, printing each and checking exit codes.
pub fn run_hooks(phase: &str, commands: &[String]) -> anyhow::Result<()>
```

- Print each command before running: `hook (pre-install): echo "Installing..."`
- Run via `std::process::Command::new("sh").arg("-c").arg(cmd)`
- If any command exits non-zero, bail with the phase name and exit code
- If the commands list is empty, do nothing (no output)
- Use `console::style` for colored output (phase name in blue)

### 3. Register module in `src/lib.rs`

Add `pub mod hooks;`

### 4. Wire hooks into `src/cli/install.rs`

In the `run()` function, add hook calls:
- Before the lock/fetch step: `hooks::run_hooks("pre-install", &pre_install_cmds)?`
- After the generate step: `hooks::run_hooks("post-install", &post_install_cmds)?`

Extract the hook commands from the manifest:
```rust
let hook_cmds = manifest.hooks.as_ref();
let pre_install = hook_cmds.map(|h| h.pre_install.as_slice()).unwrap_or_default();
// etc.
```

### 5. Wire hooks into `src/cli/generate.rs`

- Before generation: `hooks::run_hooks("pre-generate", ...)?`
- After generation: `hooks::run_hooks("post-generate", ...)?`

### Tests

In `src/hooks.rs`:
- Test running a successful command (`echo hello`)
- Test running a failing command (exit code propagation)
- Test empty commands list (no-op)
- Test multiple commands run in order

In `src/manifest/types.rs`:
- Test parsing manifest with hooks section
- Test hooks default to empty when omitted

## Important Notes
- Only modify: `src/manifest/types.rs`, `src/lib.rs`, `src/hooks.rs` (new), `src/cli/install.rs`, `src/cli/generate.rs`
- Do NOT modify `src/cli/mod.rs` (no new CLI command — hooks are wired into existing commands)
- Use `source "$HOME/.cargo/env" && cargo ...` to run cargo commands
