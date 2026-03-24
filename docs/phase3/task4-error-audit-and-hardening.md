# Phase 3, Task 4: Error Message Audit + Edge Case Hardening

## Error Message Audit

Review all user-facing error messages across the CLI handlers and ensure they are:
1. Clear and actionable (tell the user what to do)
2. Consistent in style (lowercase first word unless proper noun, no trailing period)
3. Include suggestions where appropriate (`did you mean...?`, `run agx install first`)

### Files to audit
- All files in `src/cli/` — every `bail!()`, `anyhow!()`, `eprintln!()` call
- `src/manifest/validate.rs` — `ValidationError` display messages
- `src/resolver/semver_utils.rs` — `ResolverError` display messages

### Common patterns to look for and fix
- Missing context on file I/O errors (should include file path)
- Generic "failed" messages without actionable advice
- Inconsistent capitalization or punctuation
- Error messages that don't explain what to do next

## Edge Case Hardening

### 1. Offline mode awareness
In `src/cli/install.rs` and `src/cli/lock.rs`: when git clone/fetch fails, check `config::is_offline()` and print a more helpful message: "network operations disabled (AGX_OFFLINE=1); ensure all dependencies are cached".

### 2. Corrupt/missing cache handling
In `src/cache/mod.rs`:
- `get_plugin()`, `get_skill()`, `get_agent()`: if the cached directory exists but is empty, treat it as missing (return None) instead of returning a path to an empty dir.

### 3. Manifest file encoding
In `src/manifest/mod.rs` (the `load()` function): add a check that the file is valid UTF-8 before parsing.
If not, return a clear error: "manifest file is not valid UTF-8".

### 4. Lockfile version check
In `src/lockfile/mod.rs` (the `load()` function): after deserializing, check `lockfile_version`.
If it's higher than the supported version (1), return an error: "lockfile was created by a newer version of agx; please upgrade".

### 5. Empty manifest name
In `src/manifest/validate.rs`: add a validation that `name` is not empty and contains only valid characters (alphanumeric, hyphens, underscores).

## Tests

Add tests for each hardening item:
- Empty cache directory returns None
- Non-UTF-8 manifest file returns clear error
- Future lockfile version returns upgrade error
- Empty/invalid project name validation
- Each improved error message (where testable)

## Important Notes
- Don't add new dependencies
- Keep changes minimal — fix messages and add guards, don't restructure
- All error message changes should preserve existing test assertions (update test expectations if needed)
