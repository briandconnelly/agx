# Phase 5, Task 3: GitHub Actions CI + Cross-Compilation

## Overview

Create a GitHub Actions CI pipeline that builds, tests, and produces cross-compiled release binaries.

## Implementation

### 1. Create `.github/workflows/ci.yml`

Triggered on push and PR to main:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    name: Test
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-targets
      - run: cargo clippy -- -D warnings

  fmt:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt -- --check
```

### 2. Create `.github/workflows/release.yml`

Triggered on tags matching `v*`:

```yaml
name: Release

on:
  push:
    tags: [v*]

permissions:
  contents: write

jobs:
  build:
    name: Build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
            binary: agx
            archive: agx-x86_64-linux.tar.gz
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
            binary: agx
            archive: agx-aarch64-linux.tar.gz
          - target: x86_64-apple-darwin
            os: macos-latest
            binary: agx
            archive: agx-x86_64-macos.tar.gz
          - target: aarch64-apple-darwin
            os: macos-latest
            binary: agx
            archive: agx-aarch64-macos.tar.gz
          - target: x86_64-pc-windows-msvc
            os: windows-latest
            binary: agx.exe
            archive: agx-x86_64-windows.zip
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - name: Install cross-compilation tools (Linux aarch64)
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-aarch64-linux-gnu
      - name: Build
        run: cargo build --release --target ${{ matrix.target }}
        env:
          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER: aarch64-linux-gnu-gcc
      - name: Package (Unix)
        if: runner.os != 'Windows'
        run: tar czf ${{ matrix.archive }} -C target/${{ matrix.target }}/release ${{ matrix.binary }}
      - name: Package (Windows)
        if: runner.os == 'Windows'
        run: Compress-Archive -Path target/${{ matrix.target }}/release/${{ matrix.binary }} -DestinationPath ${{ matrix.archive }}
      - uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.archive }}
          path: ${{ matrix.archive }}

  release:
    name: Create Release
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          merge-multiple: true
      - uses: softprops/action-gh-release@v2
        with:
          files: |
            *.tar.gz
            *.zip
          generate_release_notes: true
```

### 3. Create install script: `install.sh`

A curl-able install script at the project root:

```bash
#!/bin/sh
# Install agx — Agent Environment Manager
# Usage: curl -sSf https://raw.githubusercontent.com/<owner>/agx/main/install.sh | sh
set -e

REPO="<owner>/agx"
BINARY="agx"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  linux) OS="linux" ;;
  darwin) OS="macos" ;;
  *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# Get latest release tag
LATEST=$(curl -sSf "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')

ARCHIVE="${BINARY}-${ARCH}-${OS}.tar.gz"
URL="https://github.com/$REPO/releases/download/$LATEST/$ARCHIVE"

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"

echo "Downloading $BINARY $LATEST for $OS/$ARCH..."
curl -sSfL "$URL" | tar xz -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/$BINARY"

echo "Installed $BINARY to $INSTALL_DIR/$BINARY"
echo "Make sure $INSTALL_DIR is in your PATH"
```

## Important Notes
- Only create new files: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `install.sh`
- Do NOT modify any Rust source files
- Use placeholder `<owner>/agx` for the GitHub repo — user will replace later
- Make install.sh executable
