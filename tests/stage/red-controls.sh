#!/usr/bin/env bash
# Red controls for .github/scripts/check-stage.sh (DIRECTIVE-NXTG-20261014-16 item 1, .64 step 2).
# Each case copies the stage surfaces into a scratch dir, breaks one thing, and requires check-stage
# to go red. The unmodified copy must stay green, so a red result means the mutation was caught.
# Usage: bash tests/stage/red-controls.sh
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK=$ROOT/.github/scripts/check-stage.sh
fail=0
OLD='**Stage: Public preview.** Forge is public: anyone may install and use it. Its maturity is below alpha (pre-alpha): no SLA, and interfaces may change without notice.'

# case <name> <want: green|red> <mutation, run inside the scratch dir>
case_() {
  local name=$1 want=$2 mutation=$3 dir got
  dir=$(mktemp -d)
  cp "$ROOT"/{STAGE,README.md,CHANGELOG.md,server.json} "$dir"/
  (cd "$dir" && eval "$mutation")
  if (cd "$dir" && bash "$CHECK" >/dev/null 2>&1); then got=green; else got=red; fi
  rm -rf "$dir"
  if [ "$got" = "$want" ]; then echo "  ok   $name: $got"; else echo "  FAIL $name: $got, want $want"; fail=1; fi
}
export OLD

case_ "unchanged"                                green ':'
case_ "old pre-alpha line restored at line 9"    red   'python3 -c "import os,sys;p=\"README.md\";L=open(p,newline=\"\").read().split(\"\n\");L[8]=os.environ[\"OLD\"];open(p,\"w\",newline=\"\").write(\"\n\".join(L))"'
case_ "old sentence kept below the new line"     red   'python3 -c "import os;p=\"README.md\";L=open(p,newline=\"\").read().split(\"\n\");L.insert(9,\"Its maturity is below alpha (pre-alpha): no SLA.\");open(p,\"w\",newline=\"\").write(\"\n\".join(L))"'
case_ "public preview sentence left elsewhere"   red   'printf "\nForge is a public preview: anyone may install and use it.\n" >> README.md'
case_ "FINAL A off by one byte"                  red   'sed -i "9s/3 of 3/3 of 4/" README.md'
case_ "STAGE back to pre-alpha"                  red   'sed -i "s/^maturity: alpha/maturity: pre-alpha/" STAGE'
case_ "badge left at public_preview"             red   'sed -i "s|badge/stage-alpha-|badge/stage-public_preview-|" README.md'

[ "$fail" = 0 ] && echo "stage red controls: all pass" || echo "stage red controls: FAILED"
exit "$fail"
