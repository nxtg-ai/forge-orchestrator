#!/bin/sh
# install.sh and the Claude Code companion plugin (DIRECTIVE-NXTG-20261014-12 Part O).
# Each case runs the real installer (it downloads the latest release) in an empty HOME, with a stub
# `claude` that only records its arguments, so nothing touches a real Claude config.
# Usage: sh tests/install/companion.sh [path/to/install.sh]   (default: ./install.sh)
set -u
INSTALL_SH=$(cd "$(dirname "${1:-install.sh}")" && pwd)/$(basename "${1:-install.sh}")
WANT="plugin install nxtg-forge-orchestrator --marketplace nxtg-ai/forge-plugin"
CMD="claude $WANT"
fail=0
ok()  { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fail=1; }

# run <case> <claude: none|ok|fails> [env...] -> sets $home $out $rc
run() {
  name=$1 mode=$2; shift 2
  home=$(mktemp -d) stub=$(mktemp -d)
  if [ "$mode" != none ]; then
    rc_stub=0; [ "$mode" = fails ] && rc_stub=1
    printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s/claude-calls"\nexit %s\n' "$home" "$rc_stub" > "$stub/claude"
    chmod +x "$stub/claude"
  fi
  out=$(env -i HOME="$home" PATH="$stub:/usr/bin:/bin" FORGE_NO_MODIFY_PATH=1 "$@" sh "$INSTALL_SH" 2>&1)
  rc=$?
  echo "== $name (claude: $mode) rc=$rc"
}
calls() { if [ -f "$home/claude-calls" ]; then cat "$home/claude-calls"; fi; }
no_claude_config() {  # the installer itself must create no Claude config anywhere in HOME
  found=$(find "$home" \( -name '.claude' -o -name '.claude.json' -o -name 'settings.json' \) 2>/dev/null)
  [ -z "$found" ] && ok "no Claude config written by the installer" || bad "Claude config written: $found"
}

run "claude on PATH" ok
[ "$rc" = 0 ] && ok "exit 0" || bad "exit $rc"
[ "$(calls)" = "$WANT" ] && ok "claude called once: claude $WANT" || bad "claude calls: '$(calls)'"
no_claude_config

run "re-run (idempotent)" ok
run_twice=$(env -i HOME="$home" PATH="$stub:/usr/bin:/bin" FORGE_NO_MODIFY_PATH=1 sh "$INSTALL_SH" 2>&1); rc=$?
[ "$rc" = 0 ] && ok "second run exit 0" || bad "second run exit $rc"
[ "$(calls | sort -u)" = "$WANT" ] && ok "only the same plugin install command, never anything else" || bad "claude calls: '$(calls)'"

run "claude present but the install fails" fails
[ "$rc" = 0 ] && ok "exit 0 (forge itself is installed)" || bad "exit $rc"
echo "$out" | grep -qF "  $CMD" && ok "prints the one command" || bad "command not printed"

run "no claude on PATH" none
[ "$rc" = 0 ] && ok "exit 0" || bad "exit $rc"
echo "$out" | grep -qF "  $CMD" && ok "prints the one command" || bad "command not printed"
no_claude_config

run "opt out" ok FORGE_NO_CLAUDE_PLUGIN=1
[ "$rc" = 0 ] && ok "exit 0" || bad "exit $rc"
[ -z "$(calls)" ] && ok "claude not called" || bad "claude called: '$(calls)'"
echo "$out" | grep -qF "  $CMD" && ok "prints the one command" || bad "command not printed"

[ "$fail" = 0 ] && echo "companion: all cases pass" || echo "companion: FAILED"
exit "$fail"
