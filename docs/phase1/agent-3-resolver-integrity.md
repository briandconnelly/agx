# Phase 1 Agent 3: Version Resolver + Integrity

## Deliverables

1. **Semver constraint parsing** — parse all constraint syntaxes from the spec: exact, range, tilde, caret
2. **Version resolution** — given available versions from a registry, select the best match for a constraint
3. **Full dependency resolution** — process all plugins/skills/agents in a manifest, produce a complete lockfile
4. **Conflict detection** — detect when two plugins provide the same-named asset, respect include/exclude filters
5. **Integrity hashing** — compute SHA-256 for downloaded assets, verify on install

## Modules to Work In

- `src/resolver/mod.rs` — orchestrate full resolution
- `src/resolver/semver_utils.rs` — already has stubs, expand
- `src/cache/integrity.rs` — already implemented, may need refinements

## Interfaces to Expose

```rust
// resolver/mod.rs
pub fn resolve(
    manifest: &AgentEnvironment,
    registry_indexes: &BTreeMap<String, RegistryIndex>,
    git_client: &dyn GitClient,  // or just functions
) -> Result<Lockfile, ResolverError>;

pub fn resolve_single(
    name: &str,
    manifest: &AgentEnvironment,
    existing_lockfile: &Lockfile,
    registry_indexes: &BTreeMap<String, RegistryIndex>,
) -> Result<Lockfile, ResolverError>;

// resolver/semver_utils.rs (already stubbed)
pub fn parse_constraint(constraint: &str) -> Result<semver::VersionReq, ResolverError>;
pub fn satisfies(version: &semver::Version, constraint: &semver::VersionReq) -> bool;
pub fn resolve_best(available: &[semver::Version], constraint: &semver::VersionReq) -> Option<semver::Version>;

// cache/integrity.rs (already implemented)
pub fn hash_file(path: &Path) -> Result<String>;
pub fn hash_dir(path: &Path) -> Result<String>;
pub fn verify_file(path: &Path, expected: &str) -> Result<bool>;
pub fn verify_dir(path: &Path, expected: &str) -> Result<bool>;
```

## Testing Strategy

- **Semver:** Test all constraint syntaxes: `"1.2.0"`, `">=1.0.0, <2.0.0"`, `"~1.3.0"`, `"^1.3.0"`
- **Resolution:** Create mock registry indexes, resolve a manifest with multiple plugins, verify correct versions selected
- **Conflicts:** Two plugins providing same-named agent → error.
Same scenario with include/exclude → success.
- **Integrity:** Hash a temp file, verify it matches.
Corrupt the file, verify it fails.
- **Local assets:** Verify local paths are passed through without version resolution

## Dependencies on Other Agents

- Needs the `RegistryIndex` type from Agent 2, but can define its own mock for testing
- Needs lockfile types from Agent 1 (already defined in `lockfile/types.rs`)

## Integration Points

After this agent completes:
- `agx lock` can produce a complete lockfile from a manifest
- `agx install` can verify integrity of cached assets
- `agx check` can detect version conflicts
