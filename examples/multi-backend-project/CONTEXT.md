# Project Context

This is a Rust CLI application for data processing.

## Architecture

- **Language**: Rust (2021 edition)
- **Build**: Cargo with workspace support
- **Testing**: Built-in test framework with `cargo test`

## Conventions

- Use `thiserror` for library error types, `anyhow` for binary error handling.
- All public types and functions have doc comments.
- Prefer iterators over manual loops where readability is not sacrificed.
- Use `BTreeMap` over `HashMap` when deterministic ordering matters.

## Directory Structure

- `src/` -- library and binary source code
- `tests/` -- integration tests
- `benches/` -- benchmarks

## Important Notes

- Run `cargo clippy` before committing.
- The CI pipeline enforces `cargo fmt --check` and `cargo clippy -- -D warnings`.
