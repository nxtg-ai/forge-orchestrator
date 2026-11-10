#!/bin/sh
# install.sh fails closed on the checksum (DIRECTIVE-NXTG-20261110-02 item 4).
# Offline: a stub curl serves a local fixture archive in place of the release asset, and plays the
# .sha256 as STUB_SHA ok (matches), bad (wrong digest) or missing (HTTP error, exit 22).
# Each case runs the installer in an empty HOME and checks its exit code, its message and whether
# a forge binary was installed. Case "checksum matches" is the control: the same setup installs.
# Usage: sh tests/install/checksum.sh [path/to/install.sh]   (default: ./install.sh)
set -u
INSTALL_SH=$(cd "$(dirname "${1:-install.sh}")" && pwd)/$(basename "${1:-install.sh}")
fail=0
ok()  { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fail=1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The fixture archive, named exactly as install.sh names this host's asset.
case "$(uname -s)" in Linux) plat=linux ;; Darwin) plat=macos ;; *) echo "checksum: unsupported OS"; exit 1 ;; esac
case "$(uname -m)" in x86_64|amd64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;; *) echo "checksum: unsupported arch"; exit 1 ;; esac
ARCHIVE="forge-$plat-$arch.tar.gz"
mkdir -p "$WORK/pkg" "$WORK/fixture"
printf '#!/bin/sh\necho "forge 0.0.0-fixture"\n' > "$WORK/pkg/forge"
chmod +x "$WORK/pkg/forge"
tar czf "$WORK/fixture/$ARCHIVE" -C "$WORK/pkg" forge
if command -v sha256sum >/dev/null 2>&1; then
  sum=$(cd "$WORK/fixture" && sha256sum "$ARCHIVE")
else
  sum=$(cd "$WORK/fixture" && shasum -a 256 "$ARCHIVE")
fi
printf '%s\n' "$sum" > "$WORK/fixture/good.sha256"
printf '%064d  %s\n' 0 "$ARCHIVE" > "$WORK/fixture/bad.sha256"

mkdir -p "$WORK/stub"
cat > "$WORK/stub/curl" <<STUBEOF
#!/bin/sh
# Stub curl: writes the fixture named by the URL to the -o path. An HTTP error exits 22 (curl -f).
out="" url=""
while [ \$# -gt 0 ]; do
  case "\$1" in -o) out=\$2; shift 2 ;; -*) shift ;; *) url=\$1; shift ;; esac
done
echo "\$url" >> "\$HOME/curl-calls"
case "\$url" in
  */$ARCHIVE) cp "$WORK/fixture/$ARCHIVE" "\$out" ;;
  */$ARCHIVE.sha256)
    case "\${STUB_SHA:-ok}" in
      ok)  cp "$WORK/fixture/good.sha256" "\$out" ;;
      bad) cp "$WORK/fixture/bad.sha256" "\$out" ;;
      *)   echo "curl: (22) The requested URL returned error: 404" >&2; exit 22 ;;
    esac ;;
  *) echo "curl: (22) The requested URL returned error: 404" >&2; exit 22 ;;
esac
STUBEOF
chmod +x "$WORK/stub/curl"

# A PATH with every tool from /usr/bin and /bin except the SHA-256 tools.
mkdir -p "$WORK/nosha"
for d in /usr/bin /bin; do
  for t in "$d"/*; do
    n=$(basename "$t")
    case "$n" in shasum|sha256sum|shasum5*) continue ;; esac
    [ -e "$WORK/nosha/$n" ] || [ -L "$WORK/nosha/$n" ] || ln -s "$t" "$WORK/nosha/$n"
  done
done

# run <case> <path-after-stub> [VAR=value...] -> $home $out $rc
run() {
  name=$1 path=$2; shift 2
  home=$(mktemp -d "$WORK/home.XXXXXX")
  out=$(env -i HOME="$home" PATH="$WORK/stub:$path" FORGE_NO_MODIFY_PATH=1 FORGE_NO_CLAUDE_PLUGIN=1 "$@" sh "$INSTALL_SH" 2>&1)
  rc=$?
  echo "== $name rc=$rc"
}
installed() { [ -e "$home/.local/bin/forge" ]; }
expect_abort() {  # <message fragment>
  [ "$rc" != 0 ] && ok "exits non-zero" || bad "exits 0"
  installed && bad "installed forge anyway" || ok "installed nothing"
  printf '%s\n' "$out" | grep -qF "$1" && ok "says: $1" || bad "message missing: $1"
}

run "checksum matches (control)" /usr/bin:/bin STUB_SHA=ok
[ "$rc" = 0 ] && ok "exit 0" || bad "exit $rc"
installed && ok "installed forge" || bad "forge not installed"
printf '%s\n' "$out" | grep -qF "Verifying checksum..." && ok "verifies the checksum" || bad "no checksum line"

run "checksum file missing" /usr/bin:/bin STUB_SHA=missing
expect_abort "could not download the checksum file"

run "no SHA-256 tool" "$WORK/nosha" STUB_SHA=ok
expect_abort "no SHA-256 tool found"

run "checksum mismatch" /usr/bin:/bin STUB_SHA=bad
expect_abort "FAILED"

[ "$fail" = 0 ] && echo "checksum: PASS" || echo "checksum: FAIL"
exit "$fail"
