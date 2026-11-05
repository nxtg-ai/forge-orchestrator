#!/usr/bin/env bash
# The runnable examples work against the built binary and one temp project:
# mcp_stdio creates a task through `forge mcp`, task_board reads it back through the library,
# capture_knowledge writes and lists a decision. Each example must exit 0 within the timeout, and
# each checked line must match a whole output line.
# Usage: bash tests/examples/run.sh <path/to/forge>
#   EXAMPLES_RUNNER   how an example is run (default: cargo run -q --locked --example); red controls swap it
#   EXAMPLES_TIMEOUT  seconds per example (default 120), so a mute server cannot hold CI
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE=$(cd "$(dirname "${1:?usage: run.sh <path/to/forge>}")" && pwd)/$(basename "$1")
read -r -a RUNNER <<< "${EXAMPLES_RUNNER:-cargo run -q --locked --example}"
P=$(mktemp -d); trap 'rm -rf "$P"' EXIT
fail=0
out=""
run() {  # run <example> <args...>: sets $out; a non-zero exit or a timeout is a failure
  local rc
  out=$(timeout "${EXAMPLES_TIMEOUT:-120}" "${RUNNER[@]}" "$@" 2>&1); rc=$?
  if [ "$rc" = 124 ]; then echo "  FAIL $1 timed out after ${EXAMPLES_TIMEOUT:-120}s"; fail=1
  elif [ "$rc" != 0 ]; then echo "  FAIL $1 exited $rc"; printf '%s\n' "$out" | tail -3 | sed 's/^/       /'; fail=1
  else echo "  ok   $1 exited 0"; fi
}
expect() {  # expect <name> <whole output line>
  if printf '%s\n' "$out" | grep -qxF -- "$2"; then echo "  ok   $1"; else echo "  FAIL $1 (want the line: $2)"; fail=1; fi
}
run mcp_stdio "$FORGE" "$P"
expect "mcp_stdio: server answers initialize" "server: forge-mcp $("$FORGE" --version | awk '{print $2}')"
expect "mcp_stdio: lists the 12 tools" "12 tools:"
expect "mcp_stdio: creates a task" "created T-001 (pending)"
run task_board "$P"
expect "task_board: shows the task from .forge/" "  T-001 Parse the config"
run capture_knowledge "$P"
expect "capture_knowledge: lists one decision" "decisions: 1"
[ "$fail" = 0 ] && echo "examples: PASS" || echo "examples: FAIL"
exit "$fail"
