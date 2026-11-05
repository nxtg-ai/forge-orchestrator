#!/usr/bin/env bash
# The stale bot never marks an outside author's issue or PR stale (emma-soul al:66c6c92e0cfbf5a2).
# 1. community-label.py picks exactly the outside, non-bot authors not yet labelled (fixture below).
# 2. stale.yml runs that labelling before actions/stale and exempts `community` for issues and PRs.
# Usage: bash tests/stale/community.sh
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
fail=0
ok()  { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fail=1; }
got=$(python3 .github/scripts/community-label.py tests/stale/fixtures/open-items.jsonl | paste -sd' ')
# 101 NONE, 102 CONTRIBUTOR, 103 FIRST_TIME_CONTRIBUTOR. Not: OWNER, MEMBER, COLLABORATOR, a bot,
# or an item that already has the label.
if [ "$got" = "101 102 103" ]; then ok "labels outside authors only: $got"; else bad "labels outside authors only (got '$got', want '101 102 103')"; fi
W=.github/workflows/stale.yml
python3 - "$W" <<'PY' && ok "stale.yml labels first, then exempts community" || bad "stale.yml labels first, then exempts community"
import sys, yaml
steps = yaml.safe_load(open(sys.argv[1]))["jobs"]["stale"]["steps"]
uses = [s.get("uses", "") for s in steps]
i_stale = next(i for i, u in enumerate(uses) if u.startswith("actions/stale@"))
i_label = next(i for i, s in enumerate(steps) if "community-label.py" in s.get("run", ""))
w = steps[i_stale]["with"]
exempt = lambda k: [x.strip() for x in w[k].split(",")]
assert i_label < i_stale, "labelling must run before actions/stale"
assert "community" in exempt("exempt-issue-labels"), "issues"
assert "community" in exempt("exempt-pr-labels"), "PRs"
PY
[ "$fail" = 0 ] && echo "stale community: PASS" || echo "stale community: FAIL"
exit "$fail"
