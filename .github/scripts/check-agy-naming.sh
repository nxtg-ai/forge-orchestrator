#!/usr/bin/env bash
# The third CLI is Antigravity CLI (agy), not the retired Gemini CLI (EMMA-SOUL ruling Dx3 9c94b774,
# DIRECTIVE-NXTG-20261015-05 item 3). Fails if a current doc or the CLI help says "Gemini CLI" or
# tells a user to install or log in to the retired client.
# History is allowed, and only where marked as history:
#   - CHANGELOG.md is not scanned (history is not rewritten);
#   - README.md, inside the "## Past Result" section;
#   - the dated docs in DATED, each of which must open with a successor note.
# Usage: bash .github/scripts/check-agy-naming.sh [forge-binary]   (run from the repo root)
set -uo pipefail
fail=0
red() { echo "agy-naming: $*"; fail=1; }

DATED=(docs/research/cli-subscription-gating-analysis.md docs/VISION-v2.md)
is_dated() { local f; for f in "${DATED[@]}"; do [ "$1" = "$f" ] && return 0; done; return 1; }

for f in "${DATED[@]}"; do
  [ -f "$f" ] || continue
  head -5 "$f" | grep -q '^> \*\*Successor note' || red "$f: dated doc with no successor note in its first 5 lines"
done

mapfile -t FILES < <(ls README.md SPEC.md CLAUDE.md Cargo.toml 2>/dev/null; find docs -name '*.md' 2>/dev/null | sort)
for f in "${FILES[@]}"; do
  is_dated "$f" && continue
  # Print "<line>:<text>" for every hit outside README's Past Result section.
  hits=$(awk -v readme="$([ "$f" = README.md ] && echo 1)" '
    /^## / { past = ($0 ~ /^## Past Result/) }
    tolower($0) ~ /gemini cli|gemini login|gemini-cli|gemini -p|--yolo --sandbox=false/ {
      if (readme && past) next
      print FNR ":" $0
    }' "$f")
  [ -n "$hits" ] && while IFS= read -r h; do red "$f:${h:0:160}"; done <<< "$hits"
done

if [ $# -ge 1 ]; then
  help=$("$1" --help 2>&1)
  grep -qi 'gemini cli' <<< "$help" && red "forge --help says Gemini CLI"
  grep -q 'Antigravity CLI (agy)' <<< "$help" || red "forge --help does not name Antigravity CLI (agy)"
fi

[ "$fail" = 0 ] && echo "agy-naming: ok"
exit "$fail"
