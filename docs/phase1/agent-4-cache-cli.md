# Phase 1 Agent 4: Cache Manager + CLI Commands

## Deliverables

1. **Cache storage** — place downloaded assets in XDG-compliant global cache with content-addressed layout
2. **Cache retrieval** — look up cached assets by source + version/commit
3. **Cache cleanup** — `agx cache clean` and `agx cache clean <name>`
4. **Cache listing** — `agx cache list` with human-readable sizes
5. **CLI wiring** — implement `agx list`, `agx check`, `agx clean`, `agx cache dir/clean/list`
6. **.gitignore generation** — generate appropriate .gitignore entries during `agx init`

## Modules to Work In

- `src/cache/mod.rs` — expand with storage/retrieval functions
- `src/cli/cache_cmd.rs` — already partially implemented, finish
- `src/cli/list.rs` — load manifest + lockfile, print dependency tree
- `src/cli/check.rs` — load manifest, validate, report
- `src/cli/clean.rs` — remove .agx/ and generated backend dirs
- `src/config.rs` — may need to expand for project root detection

## Interfaces to Expose

```rust
// cache/mod.rs (expand existing)
pub fn cache_dir() -> Result<PathBuf>;           // already done
pub fn store_plugin(name: &str, version: &str, content_dir: &Path) -> Result<PathBuf>;
pub fn store_skill(name: &str, commit: &str, content_dir: &Path) -> Result<PathBuf>;
pub fn store_agent(name: &str, commit: &str, content_path: &Path) -> Result<PathBuf>;
pub fn get_plugin(name: &str, version: &str) -> Result<Option<PathBuf>>;
pub fn get_skill(name: &str, commit: &str) -> Result<Option<PathBuf>>;
pub fn get_agent(name: &str, commit: &str) -> Result<Option<PathBuf>>;
pub fn clean_all() -> Result<()>;                // already done
pub fn clean_asset(name: &str) -> Result<()>;
pub fn list_cached() -> Result<Vec<CacheEntry>>; // expand return type

pub struct CacheEntry {
    pub category: String,  // "plugins", "skills", "agents", "git"
    pub name: String,
    pub version_or_commit: String,
    pub size_bytes: u64,
    pub path: PathBuf,
}
```

## Testing Strategy

- **Cache storage/retrieval:** Store a temp directory as a plugin, retrieve it, verify contents match
- **Cache isolation:** Verify two different versions of the same plugin are stored separately
- **Clean:** Store assets, clean, verify they're gone
- **CLI output:** Test `agx list` output format with a sample manifest
- **XDG compliance:** Test with `AGX_CACHE_DIR` override

## Dependencies on Other Agents

- Uses manifest types (already defined)
- `agx check` needs validation from Agent 1
- `agx list` needs manifest loading from Agent 1

## Integration Points

After this agent completes:
- `agx cache dir` / `agx cache clean` / `agx cache list` work
- `agx clean` removes generated directories
- `agx check` validates manifests
- `agx list` shows dependency tree
- Cache is ready for `agx install` to store downloaded assets
