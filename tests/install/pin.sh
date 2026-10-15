#!/bin/sh
# install.sh installs the pinned release by default, never releases/latest (DIRECTIVE-NXTG-20261015-08).
# Offline: a stub curl records every URL. Asked for releases/latest it answers with a newer tag
# (v9.9.9), so an installer that still follows "latest" downloads v9.9.9 and fails here. The stub
# refuses every download, so nothing is installed and the installer stops at the download step.
# Usage: sh tests/install/pin.sh [path/to/install.sh]   (default: ./install.sh)
set -u
PIN=v1.7.1
INSTALL_SH=$(cd "$(dirname "${1:-install.sh}")" && pwd)/$(basename "${1:-install.sh}")
fail=0
ok()  { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fail=1; }

STUB=$(mktemp -d)
home=$(mktemp -d)
trap 'rm -rf "$STUB" "$home"' EXIT
cat > "$STUB/curl" <<'STUBEOF'
#!/bin/sh
echo "$*" >> "$HOME/curl-calls"
case "$*" in
  *releases/latest*) printf 'https://github.com/nxtg-ai/forge-orchestrator/releases/tag/v9.9.9'; exit 0 ;;
esac
exit 22
STUBEOF
chmod +x "$STUB/curl"

out=$(env -i HOME="$home" PATH="$STUB:/usr/bin:/bin" FORGE_NO_MODIFY_PATH=1 FORGE_NO_CLAUDE_PLUGIN=1 sh "$INSTALL_SH" 2>&1)
calls=$(cat "$home/curl-calls" 2>/dev/null)
echo "pin: $INSTALL_SH"
printf '%s\n' "$calls" | sed 's/^/  curl /'

if printf '%s\n' "$calls" | grep -q 'releases/latest'; then bad "asks GitHub for releases/latest"; else ok "never asks for releases/latest"; fi
first=$(printf '%s\n' "$calls" | grep -m1 'releases/download/')
case "$first" in
  *"/releases/download/$PIN/forge-"*) ok "downloads $PIN" ;;
  *) bad "downloads $PIN (first download: ${first:-none})" ;;
esac
if printf '%s\n' "$calls" | grep 'releases/download/' | grep -qv "/releases/download/$PIN/"; then
  bad "every download is a $PIN asset"
else
  ok "every download is a $PIN asset"
fi
if printf '%s\n' "$out" | grep -qF "Installing Forge ${PIN} ("; then ok "says it installs ${PIN}"; else bad "says it installs ${PIN}"; fi

[ "$fail" = 0 ] && echo "pin: PASS" || echo "pin: FAIL"
exit "$fail"
