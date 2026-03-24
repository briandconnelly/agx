# Phase 1 Agent 1: Manifest Parser + Lockfile Manager

## Deliverables

1. **Complete manifest parser** — `agx install` and `agx check` can load and validate any conformant `agent-environment.yml`
2. **Manifest writer** — `agx add` and `agx remove` can modify the manifest programmatically without destroying formatting (or at minimum, produce valid YAML)
3. **Lockfile writer** — generate a complete `agent-environment.lock` from resolved data
4. **Lockfile reader** — parse an existing lockfile and compare against manifest for staleness detection
5. **Manifest schema validation** — validate against `agent-environment.schema.json` at load time (or as part of `agx check`)

## Modules to Work In

- `src/manifest/mod.rs` — load/save with better error messages
- `src/manifest/types.rs` — already defined, may need refinements discovered during testing
- `src/manifest/validate.rs` — expand validation: check version constraint syntax, verify path references, detect circular dependencies
- `src/lockfile/mod.rs` — load/save
- `src/lockfile/types.rs` — already defined, may need refinements

## Interfaces to Expose

```rust
// manifest/mod.rs
pub fn load(path: &Path) -> Result<AgentEnvironment>;
pub fn save(manifest: &AgentEnvironment, path: &Path) -> Result<()>;

// manifest/validate.rs
pub fn validate(manifest: &AgentEnvironment) -> Result<(), Vec<ValidationError>>;
pub fn check_version_constraints(manifest: &AgentEnvironment) -> Result<()>;

// lockfile/mod.rs
pub fn load(path: &Path) -> Result<Lockfile>;
pub fn save(lockfile: &Lockfile, path: &Path) -> Result<()>;
pub fn is_stale(lockfile: &Lockfile, manifest: &AgentEnvironment) -> bool;
```

## Testing Strategy

- Unit tests in each file's `mod tests`
- Parse the full example manifest from the spec (Section 3.1)
- Round-trip test: parse → serialize → parse → assert equality
- Validate error messages for malformed manifests (missing required fields, invalid version syntax, undefined registry references)
- Test lockfile staleness detection: manifest has a plugin that's not in the lockfile
- Create test fixtures in `tests/fixtures/` with various manifest files

## Dependencies on Other Agents

None — this agent works independently.
The types in `manifest/types.rs` are already defined and are the shared contract.

## Integration Points

After this agent completes, the following commands should work end-to-end:
- `agx check` — load manifest, validate, report errors
- `agx list` — load manifest, print dependency tree
