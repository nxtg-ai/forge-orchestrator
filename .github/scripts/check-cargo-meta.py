#!/usr/bin/env python3
"""Cargo.toml [package] metadata that crates.io and docs.rs show (EMMA-SOUL Dx3 73d06ff8 item 6).

Required: description, license, repository, homepage, readme, keywords, categories.
crates.io limits: at most 5 keywords, each 1-20 characters, ASCII letters, digits, `_`, `-` or `+`,
starting with a letter; at most 5 categories, each a real crates.io slug
(.github/crates-io-categories.txt). `readme` must name a file that exists.

Usage: check-cargo-meta.py [Cargo.toml] [categories.txt]
"""
import os
import re
import sys
import tomllib

cargo = sys.argv[1] if len(sys.argv) > 1 else "Cargo.toml"
cats_file = sys.argv[2] if len(sys.argv) > 2 else ".github/crates-io-categories.txt"
pkg = tomllib.load(open(cargo, "rb")).get("package", {})
slugs = {l.strip() for l in open(cats_file, encoding="utf-8") if l.strip() and not l.startswith("#")}
errors = []
for field in ("description", "license", "repository", "homepage", "readme", "keywords", "categories"):
    if not pkg.get(field):
        errors.append(f"[package] {field} is missing or empty")
kw = pkg.get("keywords", [])
if len(kw) > 5:
    errors.append(f"{len(kw)} keywords; crates.io allows at most 5")
for k in kw:
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_+-]{0,19}", k):
        errors.append(f"keyword {k!r} breaks crates.io rules (1-20 chars, letter first, [A-Za-z0-9_+-])")
cats = pkg.get("categories", [])
if len(cats) > 5:
    errors.append(f"{len(cats)} categories; crates.io allows at most 5")
for c in cats:
    if c not in slugs:
        errors.append(f"category {c!r} is not a crates.io category slug")
for url in ("repository", "homepage", "documentation"):
    if pkg.get(url) and not pkg[url].startswith("https://"):
        errors.append(f"[package] {url} is not an https URL: {pkg[url]}")
readme = pkg.get("readme")
if isinstance(readme, str) and not os.path.isfile(os.path.join(os.path.dirname(cargo) or ".", readme)):
    errors.append(f"readme {readme!r} does not exist")
for e in errors:
    print(f"::error::{cargo}: {e}")
print(f"cargo-meta: {'FAIL' if errors else 'ok'} ({len(errors)} problem(s))")
sys.exit(1 if errors else 0)
