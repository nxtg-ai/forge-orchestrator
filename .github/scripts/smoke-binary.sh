#!/usr/bin/env bash
# Execute one packaged release binary and check it reports the expected version.
# Usage: smoke-binary.sh <archive> <version>
#   <archive>  forge-<os>-<arch>.tar.gz or .zip, with <archive>.sha256 beside it
#   <version>  e.g. 1.6.4 (no leading v)
# Run it on a host of the archive's own OS and architecture: the point is that the shipped
# artifact runs there, not that the source compiles.
set -euo pipefail
archive=$1 version=$2

# The checksum file is "<hash>  <archive>".
want=$(cut -d' ' -f1 < "$archive.sha256" | tr -d '\r')
if command -v sha256sum >/dev/null; then got=$(sha256sum "$archive" | cut -d' ' -f1)
else got=$(shasum -a 256 "$archive" | cut -d' ' -f1); fi
[ "$got" = "$want" ] || { echo "::error::$archive sha256 $got != $want"; exit 1; }
echo "sha256 ok: $got"

py=$(command -v python3 || command -v python)
dir=$(mktemp -d)
"$py" -c 'import shutil, sys; shutil.unpack_archive(sys.argv[1], sys.argv[2])' "$archive" "$dir"
bin=$dir/forge
[ -f "$bin" ] || bin=$dir/forge.exe
[ -f "$bin" ] || { echo "::error::no forge binary in $archive"; ls -la "$dir"; exit 1; }
chmod +x "$bin" 2>/dev/null || true
ls -l "$bin"
if command -v file >/dev/null; then file "$bin"; fi
case "$(uname -s)" in
  Darwin) otool -L "$bin" ;;
  Linux) ldd "$bin" || true ;;
esac

out=$("$bin" --version)
echo "$out"
case "$out" in
  "forge $version (stage: "*) echo "ok: $archive runs on $(uname -sm) and prints forge $version" ;;
  *) echo "::error::$archive printed '$out', want 'forge $version (stage: …)'"; exit 1 ;;
esac
