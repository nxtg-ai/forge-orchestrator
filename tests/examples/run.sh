#!/usr/bin/env bash
# The runnable examples work against the built binary and one temp project:
# mcp_stdio creates a task through `forge mcp`, task_board reads it back through the library,
# capture_knowledge writes and lists a decision. Each output line checked here is what a user sees.
# Usage: bash tests/examples/run.sh <path/to/forge>
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE=$(cd "$(dirname "${1:?usage: run.sh <path/to/forge>}")" && pwd)/$(basename "$1")
P=$(mktemp -d); trap 'rm -rf "$P"' EXIT
fail=0
expect() {  # expect <name> <output> <fixed string>
  if printf '%s\n' "$2" | grep -qF -- "$3"; then echo "  ok   $1"; else echo "  FAIL $1 (want: $3)"; printf '%s\n' "$2" | sed 's/^/       /'; fail=1; fi
}
out=$(cargo run -q --locked --example mcp_stdio -- "$FORGE" "$P" 2>&1)
expect "mcp_stdio: server answers initialize" "$out" "server: forge-mcp $("$FORGE" --version | awk '{print $2}')"
expect "mcp_stdio: lists the 12 tools" "$out" "12 tools:"
expect "mcp_stdio: creates a task" "$out" "created T-001 (pending)"
out=$(cargo run -q --locked --example task_board -- "$P" 2>&1)
expect "task_board: shows the task from .forge/" "$out" "  T-001 Parse the config"
out=$(cargo run -q --locked --example capture_knowledge -- "$P" 2>&1)
expect "capture_knowledge: lists one decision" "$out" "decisions: 1"
[ "$fail" = 0 ] && echo "examples: PASS" || echo "examples: FAIL"
exit "$fail"
