#!/usr/bin/env python3
"""Print the numbers of open issues and PRs that need the `community` label.

The stale bot (.github/workflows/stale.yml) must never close an outside author's issue or PR, so
it exempts the `community` label. This picks who gets that label: every author whose
author_association is not OWNER, MEMBER or COLLABORATOR, except bots, and only where the label is
not already there.

Input: one JSON object per line, as `gh api --jq` prints it from the issues API:
  {"number": 1, "author_association": "NONE", "user_type": "User", "labels": ["bug"]}
Usage: community-label.py <file.jsonl>
"""
import json
import sys

INSIDE = {"OWNER", "MEMBER", "COLLABORATOR"}


def needs_label(item):
    return (
        item.get("author_association") not in INSIDE
        and item.get("user_type") != "Bot"
        and "community" not in item.get("labels", [])
    )


def main(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and needs_label(json.loads(line)):
                print(json.loads(line)["number"])


if __name__ == "__main__":
    main(sys.argv[1])
