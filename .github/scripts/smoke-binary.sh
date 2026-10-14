#!/usr/bin/env bash
# Execute one packaged release binary and check it reports the expected version.
# Usage: smoke-binary.sh <archive> <version> [stage-file]
#   <archive>     forge-<os>-<arch>.tar.gz or .zip, with <archive>.sha256 beside it
#   <version>     e.g. 1.6.4 (no leading v)
#   <stage-file>  the STAGE file of the tagged commit (default ./STAGE); the binary must print the
#                 label derived from it exactly, and never "stage: internal" (this repo is public)
# Run it on a host of the archive's own OS and architecture: the point is that the shipped
# artifact runs there, not that the source compiles.
set -euo pipefail
archive=$1 version=$2 stage_file=${3:-STAGE}

# Expected label, derived from the two STAGE axes exactly as build.rs does.
field() { sed -n "s/^$1:[[:space:]]*//p" "$stage_file" | tr -d '\r[:space:]'; }
dist=$(field distribution) mat=$(field maturity)
case "$dist/$mat" in
  internal/*) stage=internal ;;
  public/pre-alpha|public/dogfood) stage="public preview" ;;
  public/alpha|public/beta|public/ga) stage=$mat ;;
  public/rc) stage="release candidate" ;;
  *) echo "::error::$stage_file has no valid distribution/maturity (got '$dist'/'$mat')"; exit 1 ;;
esac
want_line="forge $version (stage: $stage)"

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

out=$("$bin" --version | tr -d '\r')
echo "$out"
case "$out" in
  *"stage: internal"*) echo "::error::$archive printed '$out': a public release must not say internal"; exit 1 ;;
esac
[ "$out" = "$want_line" ] || { echo "::error::$archive printed '$out', want '$want_line'"; exit 1; }
echo "ok: $archive runs on $(uname -sm) and prints '$want_line'"
