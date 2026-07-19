#!/usr/bin/env bash
# forge-pod-unadopt — standalone rollback of a `forge pod adopt` (consolidation RFC §1.5).
#
# WHY THIS EXISTS, standalone: rollback must work when the forge binary is unavailable (archived,
# broken, mid-upgrade). It restores cosmux's tmux hooks, removes the shim, and removes the adoption
# journal using only `flock`, `rm`, and `tmux` — no forge, no jq.
#
# SAFETY: it acquires the SAME advisory lock the binary uses (`flock` on pod-adoption.lock) BEFORE
# reading or removing anything, and holds it through the whole reconciliation. An unlocked `rm`
# could race a live locked adopt/repair and resurrect authority from an in-memory snapshot
# (advisor/round-9 C5) — this cannot, because it blocks until the binary releases the lock.
#
# It reads the hook-restore file the binary wrote (pod-adoption.hooks, TSV: session<TAB>hook<TAB>
# value) so the restore target is the ONE captured original, never a second derivation.
set -euo pipefail

# Same seams the binary honours, same defaults — so production restores the live surfaces and a test
# points every path at a temp dir.
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

mkdir -p "$STATE_HOME"
# The lock file is a permanent sibling — created once, never removed by a rollback, so a lock is
# never held on an unlinked inode.
[[ -e "$LOCK" ]] || : >"$LOCK"

exec 9>"$LOCK"
# BLOCK until the lock is free. This is the round-9 C5 guarantee: if the binary holds it, we wait.
flock 9

if [[ ! -e "$JOURNAL" ]]; then
  echo "forge-pod-unadopt: no adoption journal at $JOURNAL — nothing to roll back"
  exit 0
fi

# Restore every captured hook. `<UNSET>` sentinel → the hook was originally absent → unset it.
if [[ -f "$HOOKS" ]]; then
  while IFS=$'\t' read -r session hook value || [[ -n "$session" ]]; do
    [[ -z "$session" ]] && continue
    if [[ "$value" == "$UNSET_SENTINEL" ]]; then
      tmuxc set-hook -u -t "$session" "$hook" 2>/dev/null || true
    else
      tmuxc set-hook -t "$session" "$hook" "$value" 2>/dev/null || true
    fi
  done <"$HOOKS"
fi

# Remove the shim so `cosmux` reaches the real binary again.
rm -f "$SHIM"
# Remove the restore file, then the journal LAST: while the journal exists authority stays frozen;
# once it is gone the state is `unadopted` and cosmux is the sole writer.
rm -f "$HOOKS"
rm -f "$JOURNAL"

echo "forge-pod-unadopt: rolled back — cosmux hooks restored, shim removed, journal cleared"
# Lock releases on exit (fd 9 closes).
