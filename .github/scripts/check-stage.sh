#!/usr/bin/env bash
# Release-stage consistency check (DIRECTIVE-NXTG-20261007-11; two axes since DIRECTIVE-NXTG-20261013-17).
# STAGE at the repo root is the ONE constant. It records the two registry fields
# (governance/product-maturity-registry.yml, row forge), one per line, the same format as forge-plugin:
#   distribution: internal|public
#   maturity: pre-alpha|dogfood|alpha|beta|rc|ga
# The displayed label is derived, never stored (release-stage skill §2):
#   public + pre-alpha|dogfood -> Public preview; public + alpha|beta|rc|ga -> Alpha|Beta|Release candidate|GA;
#   internal + anything -> no outward label (this repo is public, so internal is RED here).
# Every surface below must state the derived label and no other stage. Exit 1 on any mismatch.
# Usage: check-stage.sh [path/to/forge]   (pass the built binary to also check --version)
set -uo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

fail=0; checked=0
ok()  { checked=$((checked+1)); echo "  ok   $1"; }
bad() { checked=$((checked+1)); fail=$((fail+1)); echo "::error::stage mismatch: $1"; }

[ -f STAGE ] || { echo "::error::STAGE file missing"; exit 1; }
field() { sed -n "s/^$1:[[:space:]]*//p" STAGE | tr -d '\r[:space:]'; }
DIST=$(field distribution); MAT=$(field maturity)
[[ "$DIST" =~ ^(internal|public)$ ]] || { echo "::error::STAGE distribution must be internal|public, got '$DIST'"; exit 1; }
[[ "$MAT" =~ ^(pre-alpha|dogfood|alpha|beta|rc|ga)$ ]] || { echo "::error::STAGE maturity must be pre-alpha|dogfood|alpha|beta|rc|ga, got '$MAT'"; exit 1; }
if [ "$DIST" = internal ]; then
  [[ "$MAT" =~ ^(alpha|beta|rc|ga)$ ]] && { echo "::error::maturity $MAT cannot be internal (alpha and above are offered outward)"; exit 1; }
  echo "::error::STAGE says internal, but this repo is public; an internal product shows no outward label"; exit 1
fi
case "$MAT" in
  pre-alpha|dogfood) L='Public preview' ;;
  alpha) L='Alpha' ;; beta) L='Beta' ;; rc) L='Release candidate' ;; ga) L='GA' ;;
esac
SLUG=$(echo "$L" | tr 'A-Z ' 'a-z_')     # badge path segment
LOW=$(echo "$L" | tr 'A-Z' 'a-z')         # metadata and --version form
echo "STAGE = distribution $DIST, maturity $MAT -> label '$L'"

# Every label form that is not this one, plus the retired single-axis words.
ALL='Public preview|Alpha|Beta|Release candidate|GA|internal|dogfood|rc|ga'
OTHERS=$(echo "$ALL" | tr '|' '\n' | grep -vxiF "$L" | paste -sd'|')
OTHER_SLUGS=$(echo "$OTHERS" | tr 'A-Z ' 'a-z_')

has() { if tr -d '\r' < "$1" | grep -qF -- "$2"; then ok "$3"; else bad "$3 (missing: $2)"; fi; }
none() { local hits; hits=$(tr -d '\r' < "$1" | grep -niE -- "$2" || true); if [ -z "$hits" ]; then ok "$3"; else bad "$3: $hits"; fi; }

README=README.md
has  "$README" "img.shields.io/badge/stage-$SLUG-"        "README badge = stage-$SLUG"
none "$README" "badge/stage-($OTHER_SLUGS)-"              "README has no other stage badge"
has  "$README" "**Stage: $L.**"                            "README stage line"
none "$README" "\*\*Stage: ($OTHERS)\.\*\*"                "README has no other stage line"
none "$README" "not yet offered or supported for outside use|internal use only" "README does not say a public product is not offered outward"

if head -n 5 CHANGELOG.md | tr -d '\r' | grep -qxF "Stage: $L (see STAGE)"; then ok "CHANGELOG top line"; else bad "CHANGELOG top line (want 'Stage: $L (see STAGE)' in first 5 lines)"; fi
none CHANGELOG.md "^Stage: ($OTHERS) "                     "CHANGELOG has no other stage line"

# MCP Registry metadata (published from server.json on each release).
d=$(jq -r '.description' server.json)
if [[ "$d" == *"($LOW)"* ]]; then ok "server.json description"; else bad "server.json description lacks '($LOW)'"; fi
if [[ "$d" =~ \(stage:\ [a-z-]+\) ]]; then bad "server.json description still has a '(stage: …)' marker: ${BASH_REMATCH[0]}"; fi

if [ -n "${1:-}" ]; then
  v=$("$1" --version)
  if [[ "$v" == *"(stage: $LOW)" ]]; then ok "forge --version"; else bad "forge --version printed: $v (want '(stage: $LOW)')"; fi
fi

echo "stage-check: $fail red over $checked checked"
[ "$fail" -eq 0 ]
