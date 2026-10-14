#!/bin/sh
# install.sh and the Claude Code companion plugin (DIRECTIVE-NXTG-20261014-12 Part O).
# Each case runs the real installer (it downloads the latest release) in an empty HOME with a stub
# `claude` that records its arguments and plays a starting state, so no real Claude config is used.
#   STUB_STATE fresh   no marketplace, nothing installed
#              mkt     the nxtg-forge marketplace is already on disk (stale until updated)
#              old313  marketplace on disk and nxtg-forge 3.13.1 installed
#   STUB_FAIL  add | update | list | pupdate | install  makes that one step fail
# Usage: sh tests/install/companion.sh [path/to/install.sh]   (default: ./install.sh)
set -u
INSTALL_SH=$(cd "$(dirname "${1:-install.sh}")" && pwd)/$(basename "${1:-install.sh}")
ADD="plugin marketplace add nxtg-ai/forge-plugin"
UPD="plugin marketplace update nxtg-forge"
LST="plugin list --json"
PUPD="plugin update nxtg-forge@nxtg-forge"
INS="plugin install nxtg-forge-orchestrator@nxtg-forge"
fail=0
ok()  { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fail=1; }

STUB=$(mktemp -d)
cat > "$STUB/claude" <<'STUBEOF'
#!/bin/sh
echo "$*" >> "$HOME/claude-calls"
case "$*" in
  "plugin marketplace add "*) [ "${STUB_FAIL:-}" = add ] && exit 1; exit 0 ;;
  "plugin marketplace update "*) [ "${STUB_FAIL:-}" = update ] && exit 1; touch "$HOME/.stub-refreshed"; exit 0 ;;
  "plugin list --json")
    [ "${STUB_FAIL:-}" = list ] && exit 1
    if [ "${STUB_STATE:-fresh}" = old313 ]; then
      printf '[\n  {\n    "id": "nxtg-forge@nxtg-forge",\n    "version": "3.13.1"\n  }\n]\n'
    else echo "[]"; fi
    exit 0 ;;
  "plugin update "*) [ "${STUB_FAIL:-}" = pupdate ] && exit 1; exit 0 ;;
  "plugin install "*)
    [ "${STUB_FAIL:-}" = install ] && { echo "boom: install failed"; exit 1; }
    # A marketplace already on disk is stale until it is updated (forge-plugin probe f).
    if [ "${STUB_STATE:-fresh}" != fresh ] && [ ! -f "$HOME/.stub-refreshed" ]; then
      echo 'Plugin "nxtg-forge-orchestrator" not found in marketplace "nxtg-forge"'; exit 1
    fi
    exit 0 ;;
esac
exit 0
STUBEOF
chmod +x "$STUB/claude"

# run <case> <with-claude: yes|no> [VAR=value...] -> $home $out $rc
run() {
  name=$1 with=$2; shift 2
  home=$(mktemp -d)
  path=/usr/bin:/bin; [ "$with" = yes ] && path="$STUB:$path"
  out=$(env -i HOME="$home" PATH="$path" FORGE_NO_MODIFY_PATH=1 "$@" sh "$INSTALL_SH" 2>&1)
  rc=$?
  echo "== $name rc=$rc"
}
calls() { if [ -f "$home/claude-calls" ]; then cat "$home/claude-calls"; fi; }
expect_calls() { [ "$(calls)" = "$1" ] && ok "claude calls in order: $(calls | tr '\n' '|')" || bad "claude calls: '$(calls | tr '\n' '|')' want '$(printf '%s' "$1" | tr '\n' '|')'"; }
expect_rc0() { [ "$rc" = 0 ] && ok "exit 0" || bad "exit $rc"; }
expect_manual() {
  for c in "claude $ADD" "claude $UPD" "claude $PUPD" "claude $INS"; do
    printf '%s\n' "$out" | grep -qF "  $c" || { bad "manual command missing: $c"; return; }
  done
  ok "prints the manual commands"
}
no_claude_config() {
  found=$(find "$home" \( -name '.claude' -o -name '.claude.json' -o -name 'settings.json' \) 2>/dev/null)
  [ -z "$found" ] && ok "installer wrote no Claude config" || bad "Claude config written: $found"
}
changes_listed() {  # one line per change: each successful step has its own line
  n=$(printf '%s\n' "$out" | grep -c '^  - ' || true)
  [ "$n" = "$1" ] && ok "$1 change lines" || bad "change lines $n, want $1"
}

FULL_FRESH=$(printf '%s\n%s\n%s\n%s' "$ADD" "$UPD" "$LST" "$INS")
FULL_OLD=$(printf '%s\n%s\n%s\n%s\n%s' "$ADD" "$UPD" "$LST" "$PUPD" "$INS")

run "fresh machine" yes STUB_STATE=fresh
expect_rc0; expect_calls "$FULL_FRESH"; changes_listed 3; no_claude_config
printf '%s\n' "$out" | grep -q "MCP tools load in new sessions" && ok "reports success" || bad "no success line"

run "marketplace already on disk" yes STUB_STATE=mkt
expect_rc0; expect_calls "$FULL_FRESH"; changes_listed 3
printf '%s\n' "$out" | grep -q "MCP tools load in new sessions" && ok "companion installed after the refresh" || bad "install failed: $out"

run "nxtg-forge 3.13.1 installed" yes STUB_STATE=old313
expect_rc0; expect_calls "$FULL_OLD"; changes_listed 4
printf '%s\n' "$out" | grep -q "MCP tools load in new sessions" && ok "updated nxtg-forge, then installed the companion" || bad "no success line"

run "re-run on the same machine" yes STUB_STATE=old313
out=$(env -i HOME="$home" PATH="$STUB:/usr/bin:/bin" FORGE_NO_MODIFY_PATH=1 STUB_STATE=old313 sh "$INSTALL_SH" 2>&1); rc=$?
expect_rc0
[ "$(calls | sort -u | wc -l)" = 5 ] && ok "second run repeats the same 5 commands, nothing new" || bad "calls: $(calls | sort -u | tr '\n' '|')"

for step in add update list pupdate install; do
  run "failure at $step" yes STUB_STATE=old313 STUB_FAIL=$step
  expect_rc0; expect_manual
  printf '%s\n' "$out" | grep -q "FAILED" && ok "names the failed step" || bad "failure not reported"
  printf '%s\n' "$out" | grep -q "MCP tools load in new sessions" && bad "claims success" || ok "does not claim success"
done
# a failure stops the chain: nothing after the failed step runs
run "failure at add stops the chain" yes STUB_STATE=fresh STUB_FAIL=add
expect_calls "$ADD"

run "no claude on PATH" no
expect_rc0; expect_manual; no_claude_config

run "opt out" yes FORGE_NO_CLAUDE_PLUGIN=1
expect_rc0; expect_manual
[ -z "$(calls)" ] && ok "claude not called" || bad "claude called: '$(calls)'"

[ "$fail" = 0 ] && echo "companion: all cases pass" || echo "companion: FAILED"
exit "$fail"
