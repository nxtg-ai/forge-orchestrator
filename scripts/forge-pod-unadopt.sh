#!/usr/bin/env bash
# forge-pod-unadopt — standalone rollback of a `forge pod adopt` (consolidation RFC §1.5).
#
# WHY STANDALONE: rollback must work when the forge binary is unavailable (archived, broken,
# mid-upgrade). It restores cosmux's tmux hooks, removes the shim, and removes the adoption journal
# using only `flock`, `rm`, `tmux` (+ `sed`/`awk`) — no forge, no jq.
#
# SAFETY (regate-15 cures):
#   P1-1  FREEZE FIRST — flip the journal to `unadopting` (authority refuses writes) BEFORE touching
#         any hook. Store-write authority is a lock-free journal read.
#   P1-2  FAIL CLOSED per restore — every hook restore is verified by readback.
#   P1-4  RECORDED-HOOK RECONCILIATION — the journal's `steps.hooks` is the authoritative list of
#         what adopt rebound. The freeze PRESERVES that enumeration (flips only `state`, never
#         `hooks:{}`), and every recorded session must be reachable AND non-forge by live readback
#         before anything is deleted. An absent/truncated backup, or an unparseable journal, fails
#         closed — never delete-then-claim-success while a live pane-died hook is still forge-owned.
#   Same `flock` the binary uses, acquired BEFORE reading or removing anything (round-9 C5).
set -euo pipefail

STATE_HOME="${FORGE_POD_JOURNAL_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/forge}"
JOURNAL="$STATE_HOME/pod-adoption.json"
LOCK="$STATE_HOME/pod-adoption.lock"
HOOKS="$STATE_HOME/pod-adoption.hooks"
SHIM="${FORGE_POD_SHIM_PATH:-$HOME/.local/bin/cosmux}"
UNSET_SENTINEL=$'\x00UNSET'

tmuxc() {
  if [[ -n "${FORGE_POD_TMUX_SOCKET:-}" ]]; then
    tmux -L "$FORGE_POD_TMUX_SOCKET" "$@"
  else
    tmux "$@"
  fi
}

# A hook that invokes `pod _pane-recover` / `pod _after-detach` is forge-owned; cosmux's has no
# `pod ` infix. After a full rollback no session hook may be forge-owned.
is_forge_hook() { grep -qE 'pod _pane-recover|pod _after-detach'; }

# The sessions adopt recorded under steps.hooks — the authoritative "these were rebound" list.
recorded_hook_sessions() {
  awk '
    /"hooks"[[:space:]]*:[[:space:]]*\{[[:space:]]*\}/ { next }
    /"hooks"[[:space:]]*:[[:space:]]*\{/ { inh=1; next }
    inh && /\}/ { inh=0; next }
    inh { line=$0; gsub(/^[[:space:]]*"/,"",line); sub(/".*/,"",line); if (length(line)>0) print line }
  ' "$JOURNAL"
}

mkdir -p "$STATE_HOME"
[[ -e "$LOCK" ]] || : >"$LOCK"

exec 9>"$LOCK"
flock 9   # BLOCK until free (round-9 C5): if the binary holds it, we wait.

if [[ ! -e "$JOURNAL" ]]; then
  echo "forge-pod-unadopt: no adoption journal at $JOURNAL — nothing to roll back"
  exit 0
fi

# --- P1-4: a journal we cannot parse cannot tell us what to restore → fail closed, DON'T touch it.
if ! grep -q '"state"' "$JOURNAL" || ! grep -q '"hooks"' "$JOURNAL"; then
  echo "forge-pod-unadopt: FAIL — adoption journal is truncated/unparseable; preserving it and the shim" >&2
  exit 1
fi

# Capture the recorded rebinds BEFORE the freeze (freeze preserves them, but read from the original).
mapfile -t RECORDED < <(recorded_hook_sessions)

# --- P1-1: FREEZE authority, PRESERVING steps.hooks (flip only `state`). Atomic temp+rename.
TMP="$STATE_HOME/.pod-adoption.json.rollback.$$"
sed 's/"state"[[:space:]]*:[[:space:]]*"[a-z]*"/"state": "unadopting"/' "$JOURNAL" >"$TMP"
mv -f "$TMP" "$JOURNAL"

FAILED=0

# --- P1-2: restore each captured hook, verified by readback.
if [[ -f "$HOOKS" ]]; then
  while IFS=$'\t' read -r session hook value || [[ -n "${session:-}" ]]; do
    [[ -z "${session:-}" ]] && continue
    if ! tmuxc has-session -t "$session" 2>/dev/null; then
      echo "forge-pod-unadopt: FAIL — session '$session' unreachable; cannot verify restore" >&2
      FAILED=1; continue
    fi
    if [[ "$value" == "$UNSET_SENTINEL" ]]; then
      tmuxc set-hook -u -t "$session" "$hook" 2>/dev/null || { FAILED=1; echo "forge-pod-unadopt: FAIL — unset $session/$hook" >&2; continue; }
    else
      tmuxc set-hook -t "$session" "$hook" "$value" 2>/dev/null || { FAILED=1; echo "forge-pod-unadopt: FAIL — set $session/$hook" >&2; continue; }
    fi
    if tmuxc show-options -t "$session" "$hook" 2>/dev/null | is_forge_hook; then
      echo "forge-pod-unadopt: FAIL — $session/$hook still forge-owned after restore" >&2
      FAILED=1
    fi
  done <"$HOOKS"
fi

# --- P1-4: RECONCILE every RECORDED session against live readback, independent of the backup. An
# absent/truncated backup that skipped a recorded session is caught here: the session is still
# forge-owned (or unreachable) → fail closed.
for session in "${RECORDED[@]:-}"; do
  [[ -z "$session" ]] && continue
  if ! tmuxc has-session -t "$session" 2>/dev/null; then
    echo "forge-pod-unadopt: FAIL — recorded session '$session' unreachable; cannot confirm rollback" >&2
    FAILED=1; continue
  fi
  for hook in pane-died client-detached; do
    if tmuxc show-options -t "$session" "$hook" 2>/dev/null | is_forge_hook; then
      echo "forge-pod-unadopt: FAIL — recorded session '$session' $hook still forge-owned (backup absent/incomplete?)" >&2
      FAILED=1
    fi
  done
done

if [[ "$FAILED" -ne 0 ]]; then
  echo "forge-pod-unadopt: rollback INCOMPLETE — journal (frozen 'unadopting', ledger intact), backup, and shim preserved; re-run once tmux/backup are complete" >&2
  exit 1
fi

# Every recorded hook is verified non-forge. Remove the shim, then backup, then journal LAST.
rm -f "$SHIM"
rm -f "$HOOKS"
rm -f "$JOURNAL"

echo "forge-pod-unadopt: rolled back — every recorded cosmux hook restored (verified), shim removed, journal cleared"
