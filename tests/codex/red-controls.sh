#!/usr/bin/env bash
# Red controls for tests/codex/argv-check.sh: each argv below must FAIL the check, at the step named
# for it, or the check cannot catch what it exists to catch. Usage: bash tests/codex/red-controls.sh
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
(cd "$ROOT" && FORGE_CODEX_ARGV_OUT=$work/real.jsonl cargo test --locked --quiet dump_codex_argv_for_argv_check >/dev/null)
# Without -e, a failed dump would leave nothing to mutate and make every control "red" for the wrong reason.
[ -s "$work/real.jsonl" ] || { echo "red-controls: the argv dump is empty" >&2; exit 2; }
yolo=$(grep '"headless-yolo"' "$work/real.jsonl") || { echo "red-controls: no headless-yolo argv in the dump" >&2; exit 2; }
fail=0
# control <name> <step its first FAIL line must name> <python expression over argv `a`, giving the
# mutated argv>. Red for any other reason (a crash, a different step) does not count.
control() {
  python3 -c 'import json,sys; d=json.loads(sys.argv[1]); a=d["argv"]; d["argv"]=eval(sys.argv[2]); d["name"]=sys.argv[3]; print(json.dumps(d))' \
    "$yolo" "$3" "$1" > "$work/$1.jsonl"
  if bash "$ROOT/tests/codex/argv-check.sh" "$work/$1.jsonl" >"$work/$1.out" 2>&1; then
    echo "  FAIL $1: argv-check passed, want red"; fail=1; return
  fi
  first=$(grep -m1 '^ *FAIL ' "$work/$1.out" | sed 's/^ *FAIL //')
  case $first in
    "$2"*) echo "  ok   $1: red ($first)" ;;
    *) echo "  FAIL $1: red, but not at '$2' (first FAIL: ${first:-none})"; fail=1 ;;
  esac
}
# What every release up to v1.7.0 passed (src/adapters/codex.rs on 0c82bfd).
control "old --full-auto argv" "parse: codex accepts every flag" \
  '["exec", "--full-auto", "--skip-git-repo-check", a[-1]]'
# The forge server defined, but its tools not approved: the B1 failure.
control "no approve override" "mcp: forge_get_knowledge call completed" \
  '[x for i, x in enumerate(a) if "default_tools_approval_mode" not in x and not (x == "-c" and "default_tools_approval_mode" in a[i+1])]'
# A flag codex-cli 0.162.1 does not know.
control "unknown flag" "parse: codex accepts every flag" 'a[:1] + ["--bogus-flag"] + a[1:]'
[ "$fail" = 0 ] && echo "codex argv red controls: all pass" || echo "codex argv red controls: FAILED"
exit "$fail"
