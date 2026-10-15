#!/usr/bin/env bash
# Red controls for .github/scripts/check-agy-naming.sh (DIRECTIVE-NXTG-20261015-05 item 3).
# Each case copies the scanned surfaces into a scratch dir, changes one thing, and requires the
# check to give the wanted result. The unmodified copy must stay green.
# Usage: bash tests/agy/red-controls.sh
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CHECK=$ROOT/.github/scripts/check-agy-naming.sh
fail=0

# case_ <name> <want: green|red> <mutation, run inside the scratch dir> [binary arg]
case_() {
  local name=$1 want=$2 mutation=$3 bin=${4:-} dir got
  dir=$(mktemp -d)
  cp -r "$ROOT"/{README.md,SPEC.md,CLAUDE.md,Cargo.toml,docs} "$dir"/
  (cd "$dir" && eval "$mutation")
  if (cd "$dir" && bash "$CHECK" ${bin:+"$dir/$bin"} >/dev/null 2>&1); then got=green; else got=red; fi
  rm -rf "$dir"
  if [ "$got" = "$want" ]; then echo "  ok   $name: $got"; else echo "  FAIL $name: $got, want $want"; fail=1; fi
}
fake_help() {  # a stand-in binary whose --help prints $1
  printf '#!/bin/sh\necho "%s"\n' "$1" > fakeforge && chmod +x fakeforge
}

case_ "unchanged"                                  green ':'
case_ "README tagline back to Gemini CLI"          red   'sed -i "s/with Codex CLI and Antigravity CLI (\`agy\`) as task runners — single/with Codex CLI as a task runner — single/; s/zero deps\.\*\*$/zero deps. Gemini CLI support is in development.**/" README.md'
case_ "Gemini CLI in a current guide"              red   'printf "\nInstall Gemini CLI first.\n" >> docs/HUMAN-GUIDE.md'
case_ "retired login flow in a current guide"      red   'printf "\ngemini login\n" >> docs/HUMAN-GUIDE.md'
case_ "Gemini CLI outside README Past Result"      red   'sed -i "0,/^## Install/s//Gemini CLI works too.\n\n## Install/" README.md'
case_ "history line inside README Past Result"     green 'sed -i "0,/^## Past Result.*/s//&\n\nThat run drove Gemini CLI./" README.md'
case_ "dated doc loses its successor note"         red   'sed -i "/^> \*\*Successor note/d" docs/VISION-v2.md'
case_ "Gemini model name stays allowed"            green 'printf "\nThe agy default model is Gemini 3.1 Pro.\n" >> docs/HUMAN-GUIDE.md'
case_ "help text says Gemini CLI"                  red   'fake_help "Forge orchestrates Claude Code, Codex CLI, Gemini CLI"' fakeforge
case_ "help text names Antigravity CLI (agy)"      green 'fake_help "with Codex CLI and Antigravity CLI (agy) as task runners"' fakeforge

[ "$fail" = 0 ] && echo "agy naming red controls: all pass" || echo "agy naming red controls: FAILED"
exit "$fail"
