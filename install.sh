#!/bin/sh
# Forge Orchestrator Installer
# Usage: curl -fsSL https://forge.nxtg.ai/install.sh | sh

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

# Get latest release tag from the releases/latest redirect (github.com/.../releases/tag/vX.Y.Z).
# No api.github.com call: its unauthenticated limit (60/hour per IP) runs out on shared IPs.
echo "Detecting latest version..."
LATEST=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null | sed -n 's|.*/releases/tag/\(v[0-9][^/?#]*\)$|\1|p')

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

# Make forge resolvable in new shells (the rustup/uv approach): add one guarded PATH line to the
# shell startup files. Opt out with FORGE_NO_MODIFY_PATH=1. Re-running adds nothing twice.
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    if [ -n "${FORGE_NO_MODIFY_PATH:-}" ]; then
      echo ""
      echo "NOTE: $INSTALL_DIR is not in your PATH (FORGE_NO_MODIFY_PATH is set, so no shell file was changed)."
      echo "Add it with:  export PATH=\"$INSTALL_DIR:\$PATH\""
    else
      PATH_LINE="case \":\$PATH:\" in *\":$INSTALL_DIR:\"*) ;; *) export PATH=\"$INSTALL_DIR:\$PATH\" ;; esac  # added by forge installer"
      # ~/.profile: login shells (created if missing). bash reads ~/.bash_profile or ~/.bash_login
      # instead when one exists, so those get the line too. ~/.bashrc: interactive bash.
      # ~/.zshenv: every zsh.
      RC_FILES="$HOME/.profile"
      for f in "$HOME/.bash_profile" "$HOME/.bash_login"; do
        [ -f "$f" ] && RC_FILES="$RC_FILES $f"
      done
      if [ -f "$HOME/.bashrc" ] || command -v bash >/dev/null 2>&1; then
        RC_FILES="$RC_FILES $HOME/.bashrc"
      fi
      if [ -f "$HOME/.zshenv" ] || [ -f "$HOME/.zshrc" ] || command -v zsh >/dev/null 2>&1; then
        RC_FILES="$RC_FILES $HOME/.zshenv"
      fi
      UPDATED=""
      for f in $RC_FILES; do
        if [ -f "$f" ] && grep -qF "# added by forge installer" "$f"; then
          continue
        fi
        printf '\n%s\n' "$PATH_LINE" >> "$f" && UPDATED="$UPDATED $f"
      done
      echo ""
      if [ -n "$UPDATED" ]; then
        echo "Added $INSTALL_DIR to your PATH in:$UPDATED"
      else
        echo "$INSTALL_DIR is already added to your PATH in your shell startup files."
      fi
      echo "Open a new terminal, or run this to use forge now:"
      echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
      if [ "$(basename "${SHELL:-sh}")" = fish ]; then
        echo "fish users: fish_add_path $INSTALL_DIR"
      fi
    fi
    ;;
esac

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

# Claude Code: the companion plugin nxtg-forge-orchestrator registers forge's MCP server
# (orchestrator-mcp). Everything goes through the claude CLI; this script writes no Claude config
# by any other path, and prints one line per change. Opt out with FORGE_NO_CLAUDE_PLUGIN=1.
MKT_SOURCE="nxtg-ai/forge-plugin"
MKT="nxtg-forge"
MAIN_PLUGIN="nxtg-forge@$MKT"
COMPANION="nxtg-forge-orchestrator@$MKT"
claude_manual() {
  echo "  claude plugin marketplace add $MKT_SOURCE"
  echo "  claude plugin marketplace update $MKT"
  echo "  claude plugin update $MAIN_PLUGIN   # only if nxtg-forge is already installed"
  echo "  claude plugin install $COMPANION"
}
# claude_step <what it changes> <claude args...>: one line per step; on failure, its last output line.
claude_step() {
  what=$1; shift
  if step_out=$(claude "$@" </dev/null 2>&1); then
    echo "  - $what"
  else
    echo "  - FAILED: $what ($(printf '%s\n' "$step_out" | tail -1))"
    return 1
  fi
}
echo ""
if [ -n "${FORGE_NO_CLAUDE_PLUGIN:-}" ]; then
  echo "Claude Code plugin skipped (FORGE_NO_CLAUDE_PLUGIN is set). To add forge's MCP tools later, run:"
  claude_manual
elif ! command -v claude >/dev/null 2>&1; then
  echo "Claude Code not found. To use forge's MCP tools in Claude Code, install it, then run:"
  claude_manual
else
  echo "Adding forge's MCP tools to Claude Code:"
  if claude_step "marketplace $MKT added ($MKT_SOURCE; no change if already there)" plugin marketplace add "$MKT_SOURCE" &&
    claude_step "marketplace $MKT refreshed" plugin marketplace update "$MKT" &&
    { installed=$(claude plugin list --json </dev/null 2>/dev/null) ||
      { echo "  - FAILED: listing installed Claude Code plugins"; false; }; } &&
    {
      # An older nxtg-forge still registers its own orchestrator-mcp; update it so the companion's
      # copy is the only one.
      if printf '%s' "$installed" | grep -q "\"id\": *\"$MAIN_PLUGIN\""; then
        claude_step "plugin $MAIN_PLUGIN updated" plugin update "$MAIN_PLUGIN"
      fi
    } &&
    claude_step "plugin $COMPANION installed" plugin install "$COMPANION"; then
    echo "Claude Code: forge's MCP tools load in new sessions (restart any open session)."
  else
    echo "Forge itself is installed. To finish adding its MCP tools to Claude Code, run:"
    claude_manual
  fi
fi
