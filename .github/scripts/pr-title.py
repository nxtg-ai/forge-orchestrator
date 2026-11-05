#!/usr/bin/env python3
"""A PR title must be a conventional commit subject: squash merges make it the commit on main,
and the CHANGELOG is generated from those subjects (.github/scripts/changelog.py).

Usage: pr-title.py "<title>"
"""
import re
import sys

TYPES = "feat|fix|perf|refactor|docs|revert|ci|test|build|chore|style|release"
PATTERN = re.compile(rf"^({TYPES})(\([a-z0-9._/-]+\))?!?: \S.*$")

title = sys.argv[1] if len(sys.argv) > 1 else ""
if PATTERN.match(title):
    print(f"pr-title: ok: {title}")
    sys.exit(0)
print(f"::error::PR title is not a conventional commit subject: {title!r}. "
      f"Use <type>(<scope>)?: <description>, type one of {TYPES.replace('|', ', ')}.")
sys.exit(1)
