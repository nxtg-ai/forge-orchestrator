#!/usr/bin/env bash
# Red controls for tests/examples/run.sh: it must fail when an example prints every expected line
# and then exits non-zero, when an example hangs, and when a count only starts with the right digit.
# Each control swaps in a fake runner (EXAMPLES_RUNNER) that plays one example's output.
# Usage: bash tests/examples/red-controls.sh <path/to/forge>
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE=${1:?usage: red-controls.sh <path/to/forge>}
V=$("$FORGE" --version | awk '{print $2}')
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fail=0
fake() {  # fake <file> <exit code> <capture_knowledge decisions line> [sleep seconds]
  cat > "$1" <<FAKE
#!/bin/sh
case "\$1" in
  mcp_stdio) printf 'server: forge-mcp $V\n12 tools:\ncreated T-001 (pending)\n' ;;
  task_board) printf 'Pending (1)\n  T-001 Parse the config\n' ;;
  capture_knowledge) [ -n "${4:-}" ] && sleep ${4:-0}; printf '%s\n' "$3" ;;
esac
[ "\$1" = capture_knowledge ] && exit $2
exit 0
FAKE
  chmod +x "$1"
}
red() {  # red <name> <expected text> <env...>
  local name=$1 want=$2; shift 2
  local out rc
  out=$(env "$@" bash tests/examples/run.sh "$FORGE" 2>&1); rc=$?
  if [ "$rc" = 1 ] && printf '%s' "$out" | grep -qF -- "$want"; then echo "  ok   $name"
  else echo "  FAIL $name (rc=$rc, want exit 1 with: $want)"; printf '%s\n' "$out" | tail -4; fail=1; fi
}
fake "$T/good" 0 "decisions: 1"
if EXAMPLES_RUNNER="$T/good" bash tests/examples/run.sh "$FORGE" >/dev/null 2>&1; then echo "  ok   the fake runner passes when every example behaves"
else echo "  FAIL the fake runner passes when every example behaves"; fail=1; fi
fake "$T/exit1" 1 "decisions: 1"
red "full output, then exit 1" "capture_knowledge exited 1" EXAMPLES_RUNNER="$T/exit1"
fake "$T/hang" 0 "decisions: 1" 30
red "an example that hangs" "capture_knowledge timed out after 2s" EXAMPLES_RUNNER="$T/hang" EXAMPLES_TIMEOUT=2
fake "$T/ten" 0 "decisions: 10"
red "decisions: 10 is not decisions: 1" "want the line: decisions: 1" EXAMPLES_RUNNER="$T/ten"
[ "$fail" = 0 ] && echo "examples red controls: all pass" || echo "examples red controls: FAILED"
exit "$fail"
