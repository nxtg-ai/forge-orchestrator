#!/bin/sh
# Forge Orchestrator Installer
# Usage: curl -sSL https://raw.githubusercontent.com/nxtg-ai/forge-orchestrator/main/install.sh | sh

set -e

REPO="nxtg-ai/forge-orchestrator"
BINARY_NAME="forge"
INSTALL_DIR="${FORGE_INSTALL_DIR:-$HOME/.local/bin}"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  PLATFORM="linux" ;;
  Darwin) PLATFORM="macos" ;;
  *)
    echo "Error: Unsupported OS: $OS"
    echo "For Windows, download from: https://github.com/$REPO/releases/latest"
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

ARCHIVE="forge-${PLATFORM}-${ARCH}.tar.gz"

# Get latest release tag
echo "Detecting latest version..."
LATEST=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | head -1 | cut -d'"' -f4)

if [ -z "$LATEST" ]; then
  echo "Error: Could not detect latest version. Check https://github.com/$REPO/releases"
  exit 1
fi

VERSION="${LATEST#v}"
echo "Installing Forge v${VERSION} (${PLATFORM}/${ARCH})..."

# Download
URL="https://github.com/$REPO/releases/download/$LATEST/$ARCHIVE"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading $URL..."
curl -sSL "$URL" -o "$TMPDIR/$ARCHIVE"

# Verify checksum if available
SHAURL="${URL}.sha256"
if curl -sSL "$SHAURL" -o "$TMPDIR/$ARCHIVE.sha256" 2>/dev/null; then
  echo "Verifying checksum..."
  cd "$TMPDIR"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$ARCHIVE.sha256"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$ARCHIVE.sha256"
  fi
  cd - >/dev/null
fi

# Extract
echo "Extracting..."
tar xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"

# Install
mkdir -p "$INSTALL_DIR"
mv "$TMPDIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
chmod +x "$INSTALL_DIR/$BINARY_NAME"

echo ""
echo "Forge v${VERSION} installed to: $INSTALL_DIR/$BINARY_NAME"

# Check PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo ""
  echo "NOTE: $INSTALL_DIR is not in your PATH."
  echo "Add it with:"
  echo ""
  SHELL_NAME=$(basename "$SHELL")
  case "$SHELL_NAME" in
    zsh)  echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc && source ~/.zshrc" ;;
    fish) echo "  fish_add_path $INSTALL_DIR" ;;
    *)    echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc && source ~/.bashrc" ;;
  esac
fi

# Verify
if command -v forge >/dev/null 2>&1; then
  INSTALLED_VERSION=$(forge --version 2>/dev/null || echo "unknown")
  echo ""
  echo "Verified: $INSTALLED_VERSION"
  echo ""
  echo "Get started:"
  echo "  cd your-project"
  echo "  forge init"
  echo "  forge plan --generate"
  echo "  forge status"
else
  echo ""
  echo "Run 'forge --version' to verify (you may need to restart your shell)."
fi
