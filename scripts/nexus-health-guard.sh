#!/usr/bin/env bash
# nexus-health-guard.sh — skip NEXUS health-check writes when HEAD hasn't moved.
# Kill-switch: touch /tmp/forge-orchestrator-loop-kill to stop any active loop.
set -euo pipefail

KILL_SWITCH="/tmp/forge-orchestrator-loop-kill"
MARKER_FILE="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)/.asif/last-nexus-commit"

if [[ -f "$KILL_SWITCH" ]]; then
    echo "[nexus-health-guard] kill-switch present — stopping loop." >&2
    exit 1
fi

CURRENT_HEAD=$(git -C "$(dirname "$0")/.." rev-parse HEAD)

if [[ -f "$MARKER_FILE" ]] && [[ "$(cat "$MARKER_FILE")" == "$CURRENT_HEAD" ]]; then
    echo "[nexus-health-guard] HEAD unchanged ($CURRENT_HEAD) — skipping health-check write." >&2
    exit 0
fi

echo "$CURRENT_HEAD" > "$MARKER_FILE"
echo "[nexus-health-guard] HEAD moved to $CURRENT_HEAD — health-check write permitted." >&2
exit 0
