#!/usr/bin/env python3
"""CHANGELOG entries come from conventional commits (EMMA-SOUL Dx3 73d06ff8 item 6).

Squash merges make each PR title one commit subject on main, so the commits since the last
release tag are the change list. This script renders them, grouped by type, into the generated
block of CHANGELOG.md, and `--check` fails when the block differs. Prose outside the block is
free text.

Which block: when the tag v<Cargo.toml version> exists, the block under ## [Unreleased]
(`<!-- changelog:generated unreleased -->`). When it does not (a release PR has bumped the
version), the block for that version (`<!-- changelog:generated vX.Y.Z -->`), and the
Unreleased block must be empty.

On a pull request, pass --rev <base sha> and --with "<PR title> (#<number>)": the squash commit
that merge will create is the PR title plus its number. Breaking changes are marked from a `!`
after the type or a `BREAKING CHANGE:` footer in the commit body; a PR's footer is not on main
yet, so the workflow passes --with-breaking when the PR's commits carry one.

Usage: changelog.py (--check | --write) [--rev REV] [--with SUBJECT]... [--changelog CHANGELOG.md]
"""
import argparse
import re
import subprocess
import sys
import tomllib

TYPE = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]+)\))?(?P<bang>!)?: (?P<desc>.+)$")
GROUPS = [("feat", "Added"), ("fix", "Fixed"), ("perf", "Performance"), ("refactor", "Changed"),
          ("docs", "Documentation"), ("revert", "Reverted")]
SKIPPED = {"ci", "test", "build", "chore", "style", "release"}
KNOWN = {t for t, _ in GROUPS} | SKIPPED


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def tag_exists(tag):
    return subprocess.run(["git", "rev-parse", "-q", "--verify", f"refs/tags/{tag}"],
                          capture_output=True).returncode == 0


def last_tag(rev):
    return git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", rev).strip()


BREAKING_FOOTER = re.compile(r"^BREAKING[ -]CHANGE: ", re.M)


def render(commits):
    """Markdown for (subject, body) pairs, newest first. Raises on a non-conventional subject.
    A commit is breaking when its type has `!` or its body has a `BREAKING CHANGE:` footer."""
    groups = {t: [] for t, _ in GROUPS}
    for s, body in commits:
        m = TYPE.match(s)
        if not m or m.group("type") not in KNOWN:
            raise ValueError(f"not a conventional commit subject: {s!r}")
        if m.group("type") in SKIPPED:
            continue
        scope = f"**{m.group('scope')}:** " if m.group("scope") else ""
        breaking = "**BREAKING** " if m.group("bang") or BREAKING_FOOTER.search(body) else ""
        groups[m.group("type")].append(f"- {breaking}{scope}{m.group('desc')}")
    parts = [f"### {title}\n" + "\n".join(groups[t]) for t, title in GROUPS if groups[t]]
    return "\n\n".join(parts)


def block(key, body):
    inner = f"\n{body}\n" if body else "\n"
    return f"<!-- changelog:generated {key} -->{inner}<!-- changelog:end -->"


def find(text, key):
    return re.findall(r"<!-- changelog:generated " + re.escape(key) + r" -->.*?<!-- changelog:end -->", text, re.S)


def main():
    ap = argparse.ArgumentParser()
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    ap.add_argument("--rev", default="HEAD")
    ap.add_argument("--with", dest="extra", action="append", default=[])
    ap.add_argument("--with-breaking", action="store_true",
                    help="the --with commits carry a BREAKING CHANGE footer (their bodies are not on main yet)")
    ap.add_argument("--changelog", default="CHANGELOG.md")
    ap.add_argument("--cargo", default="Cargo.toml")
    a = ap.parse_args()

    version = tomllib.load(open(a.cargo, "rb"))["package"]["version"]
    released = tag_exists(f"v{version}")
    key = "unreleased" if released else f"v{version}"
    base = last_tag(a.rev)
    footer = "BREAKING CHANGE: see the PR" if a.with_breaking else ""
    commits = [(x, footer) for x in reversed(a.extra)]
    for rec in git("log", "--format=%s%x1f%b%x1e", f"{base}..{a.rev}").split("\x1e"):
        if rec.strip():
            subject, _, body = rec.strip("\n").partition("\x1f")
            commits.append((subject, body))
    subjects = [c[0] for c in commits]
    try:
        want = block(key, render(commits))
    except ValueError as e:
        print(f"::error::{e}")
        return 1

    with open(a.changelog, encoding="utf-8", newline="") as f:
        raw = f.read()
    nl = "\r\n" if "\r\n" in raw else "\n"
    text = raw.replace("\r\n", "\n")
    errors = []
    found = find(text, key)
    if len(found) != 1:
        errors.append(f"{a.changelog}: want exactly one `changelog:generated {key}` block, found {len(found)}")
    elif a.write:
        text = text.replace(found[0], want)
        with open(a.changelog, "w", encoding="utf-8", newline="") as f:
            f.write(text.replace("\n", nl))
        print(f"changelog: wrote the {key} block ({len(subjects)} commit(s) since {base})")
    elif found[0] != want:
        errors.append(f"{a.changelog}: the {key} block differs from the commits since {base}; run "
                      ".github/scripts/changelog.py --write (with --rev/--with on a PR)")
        print("--- expected ---\n" + want + "\n--- found ---\n" + found[0])
    if not released:
        empty = block("unreleased", "")
        if find(text, "unreleased") not in ([empty], []):
            errors.append(f"{a.changelog}: a release PR for v{version} leaves the Unreleased block empty")
    for e in errors:
        print(f"::error::{e}")
    print(f"changelog: {'FAIL' if errors else 'ok'} ({key}, {len(subjects)} commit(s) since {base})")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
