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
