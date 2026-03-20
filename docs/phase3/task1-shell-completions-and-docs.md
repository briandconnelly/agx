# Phase 3, Task 1: Shell Completions + CONTRIBUTING.md

## Shell Completions

Add `agx completions <shell>` subcommand that outputs shell completion scripts.

### Implementation

1. Add a `Completions` variant to `Commands` in `src/cli/mod.rs`:
   ```rust
   /// Generate shell completions
   Completions(completions::CompletionsArgs),
   ```

2. Create `src/cli/completions.rs`:
   - Args: `shell: clap_complete::Shell` (bash, zsh, fish, powershell, elvish)
   - Use `clap_complete::generate()` to write completions to stdout
   - The Cli struct is available via `Cli::command()` to generate completions

3. Add `clap_complete = "4"` to `[dependencies]` in Cargo.toml.

4. Register the module in `src/cli/mod.rs` and add dispatch.

### Tests

Add a unit test that generates completions for each shell variant and asserts non-empty output.

## CONTRIBUTING.md

Create `CONTRIBUTING.md` at the project root with:

- How to build and test (`cargo build`, `cargo test`)
- Project structure overview (point to CLAUDE.md module map)
- How to add a new backend (implement `Backend` trait in `src/backends/`, register in `get_backend()`)
- How to add a new CLI command (add to `Commands` enum, create handler file, add dispatch)
- Code style: `cargo fmt`, `cargo clippy`, doc comments on public items
- Testing conventions: unit tests in same file, integration tests in `tests/`
- PR guidelines: one logical change per PR, tests required

## Update CLAUDE.md

Update the "Current Status" line in CLAUDE.md to reflect Phase 3 completion.
