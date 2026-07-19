#!/usr/bin/env bash
# forge-pod-unadopt — standalone rollback of a `forge pod adopt` (consolidation RFC §1.5).
#
# WHY STANDALONE: rollback must work when the forge binary is unavailable (archived, broken,
# mid-upgrade). It restores cosmux's tmux hooks, removes the shim, and removes the adoption journal
# using only `flock`, `rm`, and `tmux` — no forge, no jq.
#
# SAFETY (regate-15 cures):
#   P1-1  FREEZE FIRST — write `unadopting` to the journal (atomic temp+rename) BEFORE touching any
#         hook. Store-write authority is a lock-free journal read; an `adopted` journal while we
#         restore cosmux hooks would leave forge authorized with cosmux hooks back = dual-writer.
#   P1-2  FAIL CLOSED — every hook restore is VERIFIED by readback; a failure (dead socket, missing
#         session, hook still forge-owned) preserves journal+backup+shim and exits nonzero. No
#         `|| true`, no delete-then-claim-success. Codex reproduced success-with-zero-hooks-restored
#         on a dead socket; this makes that impossible.
#   Same `flock` the binary uses, acquired BEFORE reading or removing anything (round-9 C5).
set -euo pipefail

STATE_HOME="${FORGE_POD_JOURNAL_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/forge}"
JOURNAL="$STATE_HOME/pod-adoption.json"
LOCK="$STATE_HOME/pod-adoption.lock"
HOOKS="$STATE_HOME/pod-adoption.hooks"
SHIM="${FORGE_POD_SHIM_PATH:-$HOME/.local/bin/cosmux}"
UNSET_SENTINEL=$'\x00UNSET'

# tmux socket seam: tests point at a private server, production uses the default.
tmuxc() {
  if [[ -n "${FORGE_POD_TMUX_SOCKET:-}" ]]; then
    tmux -L "$FORGE_POD_TMUX_SOCKET" "$@"
  else
    tmux "$@"
  fi
}

# A hook value that invokes `pod _pane-recover` / `pod _after-detach` is forge-owned; cosmux's has
# no `pod ` infix. After a restore the value must NOT be forge-owned.
is_forge_hook() { grep -qE 'pod _pane-recover|pod _after-detach'; }

mkdir -p "$STATE_HOME"
# Permanent sibling lock — created once, never removed by a rollback, so a lock is never held on an
# unlinked inode.
[[ -e "$LOCK" ]] || : >"$LOCK"

exec 9>"$LOCK"
flock 9   # BLOCK until free (round-9 C5): if the binary holds it, we wait.

if [[ ! -e "$JOURNAL" ]]; then
  echo "forge-pod-unadopt: no adoption journal at $JOURNAL — nothing to roll back"
  exit 0
fi

# --- P1-1: FREEZE authority before any surface change ---------------------------------------------
# Atomic temp+rename so a lock-free reader sees old-or-new, never a torn file.
NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo 1970-01-01T00:00:00Z)"
TMP="$STATE_HOME/.pod-adoption.json.rollback.$$"
printf '{"schema":1,"state":"unadopting","steps":{"shim":"pending","hooks":{},"recoveries":{}},"ts":"%s","by":"unadopt-script"}\n' \
  "$NOW" >"$TMP"
mv -f "$TMP" "$JOURNAL"

# --- P1-2: restore every captured hook, VERIFIED; any failure preserves state and exits nonzero ---
FAILED=0
if [[ -f "$HOOKS" ]]; then
  while IFS=$'\t' read -r session hook value || [[ -n "${session:-}" ]]; do
    [[ -z "${session:-}" ]] && continue

    # The session must be reachable, or we cannot verify the restore. A dead socket / missing
    # session is a FAILURE, never a silent skip — we might be looking at the wrong server while the
    # real sessions still fire forge hooks.
    if ! tmuxc has-session -t "$session" 2>/dev/null; then
      echo "forge-pod-unadopt: FAIL — session '$session' unreachable; cannot verify hook restore" >&2
      FAILED=1
      continue
    fi

    if [[ "$value" == "$UNSET_SENTINEL" ]]; then
      tmuxc set-hook -u -t "$session" "$hook" 2>/dev/null || { FAILED=1; echo "forge-pod-unadopt: FAIL — could not unset $session/$hook" >&2; continue; }
    else
      tmuxc set-hook -t "$session" "$hook" "$value" 2>/dev/null || { FAILED=1; echo "forge-pod-unadopt: FAIL — could not set $session/$hook" >&2; continue; }
    fi

    # Verified readback: the hook must not be forge-owned after the restore.
    if tmuxc show-options -t "$session" "$hook" 2>/dev/null | is_forge_hook; then
      echo "forge-pod-unadopt: FAIL — $session/$hook still forge-owned after restore" >&2
      FAILED=1
    fi
  done <"$HOOKS"
fi

if [[ "$FAILED" -ne 0 ]]; then
  echo "forge-pod-unadopt: restore INCOMPLETE — journal (frozen 'unadopting'), backup, and shim preserved; re-run once tmux is reachable" >&2
  exit 1
fi

# All restores verified. Remove the shim so `cosmux` reaches the real binary again, then the backup,
# then the journal LAST: while the journal exists authority stays frozen; once gone it is `unadopted`.
rm -f "$SHIM"
rm -f "$HOOKS"
rm -f "$JOURNAL"

echo "forge-pod-unadopt: rolled back — cosmux hooks restored (verified), shim removed, journal cleared"
# Lock releases on exit (fd 9 closes).
