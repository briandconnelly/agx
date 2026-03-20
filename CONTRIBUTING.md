# Contributing to agx

## Building and Testing

```bash
cargo build          # debug build
cargo build -r       # release build
cargo test           # run all tests
cargo clippy         # lint
cargo fmt            # format code
```

## Project Structure

See the **Module Map** in [CLAUDE.md](CLAUDE.md) for a complete overview of the codebase layout. The key idea: one file per CLI command in `src/cli/`, typed errors via `thiserror` in library code, and `anyhow::Result` in CLI handlers.

## Adding a New Backend

1. Create a new file in `src/backends/` (e.g., `src/backends/my_backend.rs`).
2. Implement the `Backend` trait defined in `src/backends/mod.rs`.
3. Register your backend in the `get_backend()` function so it can be selected at runtime.

## Adding a New CLI Command

1. Add a new variant to the `Commands` enum in `src/cli/mod.rs`.
2. Create a handler file in `src/cli/` with an `Args` struct (clap derive) and a `run()` function.
3. Register the module (`pub mod`) and add a dispatch arm in `src/cli/mod.rs`.

## Code Style

- Run `cargo fmt` before committing.
- Run `cargo clippy` and fix all warnings.
- Add `///` doc comments on all public types and functions.
- Use `BTreeMap` over `HashMap` for deterministic serialization order.

## Testing Conventions

- **Unit tests:** Place in a `#[cfg(test)] mod tests` block at the bottom of the same file.
- **Integration tests:** Place in the `tests/` directory.
- All new features and bug fixes should include tests.

## PR Guidelines

- One logical change per PR.
- Tests are required for all changes.
- Ensure `cargo test` and `cargo clippy` pass before submitting.
