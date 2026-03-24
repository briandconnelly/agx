# Phase 5, Task 1: `agx publish` — Publish Assets to a Registry

## Overview

Add `agx publish` command that packages and publishes a skill, agent, or plugin to a marketplace registry.

## How Publishing Works

A registry is a git repo with a `marketplace.json` index.
Publishing means:
1. Validate the asset to publish (must have required metadata)
2. Create a git commit in the registry repo adding/updating the entry in `marketplace.json`
3. Push the change to the registry remote

For the initial implementation, we support publishing **the current project** as a plugin to a named registry.

## Implementation

### 1. Create `src/cli/publish.rs`

```rust
#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Registry to publish to
    #[arg(long)]
    pub registry: String,

    /// Version to publish (must be valid semver)
    #[arg(long)]
    pub version: String,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,
}
```

### 2. Publish flow

1. **Load manifest** — read the project's `agent-environment.yml`
2. **Validate** — ensure the manifest has `name`, `version` (or use `--version`), `description`
3. **Find registry** — look up the registry URL from the manifest's `registries` section
4. **Get current commit** — run `git rev-parse HEAD` in the project to get the commit SHA
5. **Confirm** — unless `--yes`, prompt: "Publish {name}@{version} to registry '{registry}'? [y/N]"
6. **Clone/fetch registry** — clone the registry repo to a temp directory
7. **Update marketplace.json** — read existing index, add/update the plugin entry:
   ```json
   {
     "plugins": {
       "my-plugin": {
         "description": "...",
         "versions": {
           "1.0.0": { "commit": "abc123..." }
         }
       }
     }
   }
   ```
8. **Commit and push** — commit the change and push to the registry's remote
9. **Print success** — "published {name}@{version} to registry '{registry}'"

### 3. Validation checks

Before publishing, verify:
- Project has a git repo with a clean working tree (no uncommitted changes)
- Version doesn't already exist in the registry
- Version is valid semver
- Manifest has a description

### 4. Register the command

- Add `pub mod publish;` in `src/cli/mod.rs`
- Add `Publish(publish::PublishArgs)` variant to `Commands` enum
- Add dispatch arm

### Tests

- Test validation: missing description, dirty working tree detection logic, duplicate version detection
- Test marketplace.json update logic (parse existing, add entry, serialize back)
- Test version validation (valid semver vs invalid)

## Important Notes
- Only modify: `src/cli/publish.rs` (new), `src/cli/mod.rs`
- No new dependencies — use existing serde_json, dialoguer for prompts
- The actual git push will require auth — for now, assume the user has git credentials configured
- Use `dialoguer::Confirm` for the confirmation prompt (already a dependency)
