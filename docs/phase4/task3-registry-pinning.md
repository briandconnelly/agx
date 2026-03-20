# Phase 4, Task 3: Registry Integrity Pinning

## Overview

Allow pinning a registry index to a specific git commit SHA, so even the catalog of available packages is reproducible. This prevents a compromised or modified registry from silently changing what versions are available.

## Manifest Schema

Add an optional `pin` field to `RegistryConfig`:

```yaml
registries:
  official:
    url: https://github.com/anthropics/claude-plugins-official
    pin: abc123def456789...  # optional: pin to this commit SHA
```

## Implementation

### 1. Update `src/manifest/types.rs`

Add to `RegistryConfig`:
```rust
/// Pin the registry index to a specific commit SHA for reproducibility.
#[serde(default)]
pub pin: Option<String>,
```

### 2. Update `src/manifest/validate.rs`

Add validation:
- If `pin` is set, it must be a valid hex string (40 or 64 chars, all hex digits)
- Add `InvalidRegistryPin` variant to `ValidationError`

### 3. Update `src/sources/registry.rs`

Modify `fetch_index()` (or add a wrapper) to support pinned registries:
- After fetching/cloning the registry repo, if a `pin` is provided, verify the current HEAD matches the pin SHA
- If not, try to check out the pinned commit: `git -C <path> rev-parse <pin>` to verify it exists
- When reading marketplace.json, use `git show <pin>:marketplace.json` instead of HEAD if pinned
- Add a new function: `fetch_index_pinned(url, cache_dir, pin: Option<&str>)` that wraps `fetch_index` with the pinning logic

### 4. Update `src/cli/lock.rs`

In `build_resolve_context()`, pass the registry pin to the fetch function when available:
```rust
let pin = reg_config.pin.as_deref();
let index = registry::fetch_index_pinned(&reg_config.url, &reg_dir, pin)?;
```

### 5. Add `agx registry pin` subcommand

Add a `Pin` variant to the registry subcommands in `src/cli/registry_cmd.rs`:

```rust
/// Pin a registry to its current commit SHA
Pin(RegistryPinArgs),
```

```rust
pub struct RegistryPinArgs {
    /// Registry name
    pub name: String,
}
```

Implementation:
- Find the registry in the manifest
- Look up the cached registry repo
- Get the current HEAD commit: `git -C <path> rev-parse HEAD`
- Write the pin to the manifest (update the registry's `pin` field)
- Print: "pinned registry 'official' to <sha>"

### Tests

In `src/manifest/types.rs`:
- Test parsing manifest with pinned registry
- Test pin field defaults to None

In `src/manifest/validate.rs`:
- Test valid pin (40-char hex) passes
- Test invalid pin (wrong length, non-hex) fails

In `src/sources/registry.rs`:
- Test `fetch_index_pinned` with None pin delegates to normal fetch
- (Network-dependent tests should be skipped — just test the validation logic)

In `src/cli/registry_cmd.rs`:
- Update existing registry test count if needed

## Important Notes
- Only modify: `src/manifest/types.rs`, `src/manifest/validate.rs`, `src/sources/registry.rs`, `src/cli/lock.rs`, `src/cli/registry_cmd.rs`
- Do NOT modify `src/cli/mod.rs`, `src/backends/`, or Cargo.toml
