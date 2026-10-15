#!/usr/bin/env bash
# DIRECTIVE-NXTG-20261015-07: run the Codex adapter's real argv through the pinned codex-cli, offline.
# codex-cli 0.162.1 rejected the old `--full-auto`; this check goes red if the adapter passes any flag
# or `-c` override that codex rejects, or if the unattended argv no longer lets a forge MCP call through.
# codex runs only inside a container with --network none and BROWSER=/bin/false, never on the host,
# and talks only to tests/codex/mock_responses.py. It costs $0 and uses no credential.
# Usage: bash tests/codex/argv-check.sh [argv.jsonl]   (default: dump the adapter's argv via cargo test)
# Env: FORGE_BIN (default target/debug/forge), CODEX_CACHE (default target/codex-cli-0.162.1).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CODEX_VERSION=0.162.1
IMAGE=node:22-trixie
FORGE_BIN=${FORGE_BIN:-$ROOT/target/debug/forge}
CODEX_CACHE=${CODEX_CACHE:-$ROOT/target/codex-cli-$CODEX_VERSION}
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
argvs=${1:-}
if [ -z "$argvs" ]; then
  argvs=$work/argv.jsonl
  (cd "$ROOT" && FORGE_CODEX_ARGV_OUT=$argvs cargo test --locked --quiet dump_codex_argv_for_argv_check >/dev/null)
fi
[ -s "$argvs" ] || { echo "argv-check: no argv to check" >&2; exit 2; }
[ -x "$FORGE_BIN" ] || (cd "$ROOT" && cargo build --locked --quiet)
if [ ! -x "$CODEX_CACHE/bin/codex" ]; then  # npm needs the network; codex itself never gets it
  mkdir -p "$CODEX_CACHE"
  docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$CODEX_CACHE:/opt/codex" "$IMAGE" \
    npm install --global --prefix /opt/codex --no-fund --no-audit "@openai/codex@$CODEX_VERSION" >/dev/null
fi
fail=0
while IFS= read -r line; do
  name=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("name","argv"))' "$line")
  argv=$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["argv"]))' "$line")
  echo "$name"
  docker run --rm --network none --user "$(id -u):$(id -g)" --tmpfs /home/run:exec,uid=$(id -u) --tmpfs /work:uid=$(id -u) \
    -e BROWSER=/bin/false -v "$CODEX_CACHE:/opt/codex:ro" -v "$FORGE_BIN:/usr/local/bin/forge:ro" \
    -v "$ROOT/tests/codex:/t:ro" -w /home/run "$IMAGE" python3 /t/in_container.py "$argv" || fail=1
done < "$argvs"
[ "$fail" = 0 ] && echo "argv-check: all pass" || echo "argv-check: FAILED"
exit "$fail"
