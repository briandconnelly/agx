# Phase 1 Agent 2: Git + Registry Client

## Deliverables

1. **Git clone/fetch** — clone bare repos to the global cache, fetch updates for existing clones
2. **Ref resolution** — resolve tags, branches, and "latest default branch" to exact commit SHAs
3. **Asset extraction** — extract specific files/directories from a bare repo at a pinned commit
4. **Registry index fetcher** — clone a marketplace registry repo, parse its `marketplace.json` (or equivalent index format)
5. **Registry lookup** — given a plugin name, return available versions from a cached registry index

## Modules to Work In

- `src/sources/git.rs` — already has stubs, implement fully
- `src/sources/registry.rs` — implement index fetching and lookup
- `src/cache/mod.rs` — use `git_dir()` and `registry_dir()` for storage paths

## Interfaces to Expose

```rust
// sources/git.rs
pub fn clone(url: &str, dest: &Path) -> Result<()>;
pub fn fetch(repo_path: &Path) -> Result<()>;
pub fn resolve_ref(repo_path: &Path, reference: &str) -> Result<String>;  // returns SHA
pub fn resolve_default_branch(repo_path: &Path) -> Result<String>;  // returns SHA of HEAD
pub fn extract(repo_path: &Path, commit: &str, path: &str, dest: &Path) -> Result<()>;

// sources/registry.rs
pub fn fetch_index(url: &str, cache_dir: &Path) -> Result<RegistryIndex>;
pub fn lookup_plugin(index: &RegistryIndex, name: &str) -> Option<Vec<PluginVersion>>;

// New type needed:
pub struct RegistryIndex { ... }
pub struct PluginVersion {
    pub version: semver::Version,
    pub git_commit: String,
    pub path: Option<String>,  // path within the registry repo
}
```

## Testing Strategy

- **Git operations:** Create a temporary git repo in tests using `tempfile`, push commits, test clone/fetch/resolve/extract
- **Registry:** Create a mock `marketplace.json` in a temp repo, test index parsing and lookup
- **Offline mode:** Test that operations fail gracefully when `AGX_OFFLINE=1` and cache is empty
- **Error handling:** Test invalid URLs, nonexistent refs, network failures (mock or temp repo with no remote)

## Dependencies on Other Agents

None — this agent works independently. Uses `cache::cache_dir()` which is already implemented.

## Integration Points

After this agent completes:
- `agx lock` can resolve git-based dependencies to exact SHAs
- `agx install` can download assets to the global cache

## Notes

- Shell out to `git` binary, don't use git2 crate
- The marketplace.json format needs to be defined. For now, assume a simple structure:
  ```json
  {
    "plugins": {
      "quality-review": {
        "versions": {
          "1.2.0": { "commit": "abc123", "path": "plugins/quality-review" },
          "1.1.0": { "commit": "def456", "path": "plugins/quality-review" }
        }
      }
    }
  }
  ```
- Cache bare repos at `$CACHE/git/{url-slug}.git`
- Cache registry indexes at `$CACHE/registry/{registry-name}/`
