#!/usr/bin/env bash
# Fail if any stage surface disagrees with the root STAGE file (the one constant).
# Usage: check-stage.sh [path/to/forge]   (pass the built binary to also check --version)
set -euo pipefail

stage=$(tr -d '[:space:]' < STAGE)
# Normalise line endings: CHANGELOG.md is CRLF.
readme=$(tr -d '\r' < README.md)
changelog=$(tr -d '\r' < CHANGELOG.md)
fail=0
bad() { echo "::error::stage mismatch: $1 (STAGE says '$stage')"; fail=1; }

case "$stage" in
  internal|dogfood|alpha|beta|rc|ga) ;;
  *) bad "STAGE holds '$stage', not a registry stage (internal/dogfood/alpha/beta/rc/ga)" ;;
esac

grep -qF "[![stage: $stage](https://img.shields.io/badge/stage-$stage-" <<<"$readme" \
  || bad "README badge"
grep -qF "**Stage: $stage.**" <<<"$readme" || bad "README stage line"
grep -qxF "Stage: $stage (see STAGE)" <<<"$changelog" || bad "CHANGELOG stage line"

# Any other stage word in a badge or stage line is a second, conflicting claim.
others=$(printf '%s\n%s\n' "$readme" "$changelog" \
  | grep -oE 'badge/stage-[a-z]+|\*\*Stage: [a-z]+\.\*\*|^Stage: [a-z]+ \(see STAGE\)' \
  | grep -vE "stage-$stage\$|Stage: $stage(\.\*\*| \(see STAGE\))\$" || true)
[ -z "$others" ] || bad "conflicting stage claim(s): $others"

if [ -n "${1:-}" ]; then
  "$1" --version | grep -qF "(stage: $stage)" || bad "forge --version printed: $("$1" --version)"
fi

if [ "$fail" -ne 0 ]; then exit 1; fi
echo "stage surfaces consistent: $stage"
