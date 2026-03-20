# Phase 3, Task 3: Integration Tests + Example Fixtures

## Integration Tests

Create end-to-end integration tests in `tests/` that test real workflows using the library API (not shelling out to the binary). These tests should NOT require network access.

### tests/integration_test.rs

1. **`test_full_local_workflow`** — The "golden path":
   - Create a tempdir with an `agent-environment.yml` manifest containing:
     - Two backends: claude-code, gemini-cli
     - Two local skills (create actual skill dirs with SKILL.md files)
     - One local agent (create a .md file)
     - A context section with CONTEXT.md + overlay files
   - Call `agx::manifest::load()` to parse
   - Call `agx::manifest::validate::validate()` to check validity
   - Call `lock::run_lock()` to produce lockfile
   - Verify lockfile structure (correct entries, local sources)
   - Call backend `generate()` for each backend
   - Verify generated files exist:
     - `.claude/` directory with expected contents
     - `.gemini/` directory
     - `CLAUDE.md` and `GEMINI.md` with markers
   - Call `agx::context::generate_context_files()` and verify output

2. **`test_lockfile_roundtrip`**:
   - Create lockfile via `run_lock()`
   - Save with `lockfile::save()`
   - Load with `lockfile::load()`
   - Verify contents match

3. **`test_lockfile_staleness`**:
   - Create manifest with skill A
   - Lock it
   - Modify manifest to add skill B
   - Verify `lockfile::is_stale()` returns true

4. **`test_manifest_validation_catches_errors`**:
   - Load `tests/fixtures/invalid-no-backends.yml`
   - Verify validation returns the expected error variant

5. **`test_multi_backend_generation`**:
   - Manifest with claude-code + gemini-cli + generic
   - Generate for all three backends
   - Verify each produces its expected output files

## Example Fixtures

Create `tests/fixtures/` files if they don't already exist:

1. **`tests/fixtures/full-local-project/agent-environment.yml`** — A complete manifest with all local deps:
   ```yaml
   name: example-project
   backends:
     claude-code:
       isolation: strict
       model: sonnet
     gemini-cli:
       model: gemini-2.5-pro
   skills:
     deploy-checklist:
       path: ./skills/deploy-checklist
   agents:
     code-reviewer:
       path: ./agents/code-reviewer.md
       model: opus
   context:
     base: ./CONTEXT.md
     overlays:
       claude-code: ./CONTEXT.claude.md
   ```

2. Create the supporting files in the fixture directory:
   - `skills/deploy-checklist/SKILL.md`
   - `agents/code-reviewer.md`
   - `CONTEXT.md`
   - `CONTEXT.claude.md`

## Important Notes

- Import library code via `agx::` prefix (not `crate::`)
- Use `tempfile::tempdir()` for all filesystem tests
- Copy fixture files to tempdir before testing (don't modify fixtures)
- All tests must be `#[test]` (no async, no external deps needed)
