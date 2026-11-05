#!/usr/bin/env bash
# Red controls for the docs-from-code gates (EMMA-SOUL Dx3 73d06ff8 item 6). Each case breaks one
# thing in a temp copy and requires the gate to exit 1 with the expected error. A control that
# passes, or fails for another reason, is red here.
# Usage: bash tests/docs-gates/red-controls.sh <path/to/forge>
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE=${1:?usage: red-controls.sh <path/to/forge>}
S=.github/scripts
# On a PR, CI passes the base sha, so the changelog cases see main's commits, not the PR branch's
# own commit messages. The PR's own CHANGELOG line is conventional.yml's job: it re-runs on title
# edits, which CI does not.
CL=(--rev "${CHANGELOG_REV:-HEAD}")
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fail=0
# red <name> <expected text> <command...>: the command must exit 1 and print the expected text.
red() {
  local name=$1 want=$2; shift 2
  local out rc
  out=$("$@" 2>&1); rc=$?
  if [ "$rc" = 1 ] && printf '%s' "$out" | grep -qF -- "$want"; then echo "  ok   $name"
  else echo "  FAIL $name (rc=$rc, want exit 1 with: $want)"; printf '%s\n' "$out" | tail -3; fail=1; fi
}
green() {
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then echo "  ok   $name"; else echo "  FAIL $name (want exit 0)"; fail=1; fi
}

# (a) README CLI == forge --help
green "README as committed passes" python3 $S/readme-cli.py --forge "$FORGE"
python3 - README.md "$T/r1.md" <<'PY'
import sys; s = open(sys.argv[1], encoding="utf-8").read()
start = s.index("$ forge --help\n"); line_end = s.index("\n", s.index("Commands:", start) + 10)
nxt = s.index("\n", line_end + 1)
open(sys.argv[2], "w", encoding="utf-8").write(s[:line_end] + s[nxt:])   # drop one command line
PY
red "help block missing one line" "differs from \`forge --help\`" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r1.md"
sed 's/^forge status                   # Current state summary$/forge frobnicate               # not a command/' README.md > "$T/r2.md"
red "README names a command forge does not have" "\`forge frobnicate\` is not a forge command" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r2.md"
sed 's/^forge dashboard --pty          #/forge dashboard --pty --nope   #/' README.md > "$T/r3.md"
red "README names a flag the command does not have" "\`forge dashboard --nope\` is not listed" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r3.md"

CLICOLOR_FORCE=1 green "README check ignores CLICOLOR_FORCE" python3 $S/readme-cli.py --forge "$FORGE"
sed 's/^forge init$/cd my-project \&\& forge frobnicate/' README.md > "$T/r4.md"
red "a forge call after &&" "\`forge frobnicate\` is not a forge command" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r4.md"
sed 's/^codex mcp add forge -- forge mcp$/codex mcp add forge -- forge mcp --nope/' README.md > "$T/r5.md"
red "a forge call after --" "\`forge mcp --nope\` is not listed" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r5.md"
printf '\n```\nforge frobnicate\n```\n' | cat README.md - > "$T/r6.md"
red "a forge call in a bare fence" "\`forge frobnicate\` is not a forge command" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r6.md"
# #87 item 4: forge calls the check missed before.
i=7
for call in 'sudo forge frobnicate' 'FORGE_HOME=/tmp forge frobnicate' 'echo "$(forge frobnicate)"' \
            './target/release/forge frobnicate' 'forge \' ; do
  if [ "$call" = 'forge \' ]; then body=$(printf '\n```bash\nforge \\\n  frobnicate\n```\n')
  else body=$(printf '\n```bash\n%s\n```\n' "$call"); fi
  printf '%s\n' "$body" | cat README.md - > "$T/r$i.md"
  name=$call; [ "$call" = 'forge \' ] && name='a \ line continuation'
  red "a forge call: $name" "\`forge frobnicate\` is not a forge command" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r$i.md"
  i=$((i + 1))
done
printf '\n~~~bash\nforge frobnicate\n~~~\n' | cat README.md - > "$T/r$i.md"
red "a forge call in a ~~~ fence" "\`forge frobnicate\` is not a forge command" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r$i.md"
# #87 item 2: prose in a bare fence names the fix.
printf '\n```\nforge is a tool\n```\n' | cat README.md - > "$T/r_prose.md"
red "prose in a bare fence says to label it text" "label it \`\`\`text" python3 $S/readme-cli.py --forge "$FORGE" --readme "$T/r_prose.md"

# (b) CHANGELOG == conventional commits since the last tag
if [ "${CHANGELOG_BASELINE:-run}" = skip ]; then
  echo "  skip CHANGELOG as committed passes (a PR: conventional.yml checks it with the PR title)"
else
  green "CHANGELOG as committed passes" python3 $S/changelog.py --check "${CL[@]}"
fi
# A merged fix commit with no entry, in a throwaway repo with one tagged release. It does not use
# this repo's CHANGELOG: once a release is tagged, only [Unreleased] is checked, and that can be empty.
M="$T/missing"; mkdir -p "$M" && (
  cd "$M" && git init -q && git config user.email t@example.invalid && git config user.name t
  printf '[package]\nname = "x"\nversion = "0.1.0"\n' > Cargo.toml
  printf '# Changelog\n\n<!-- changelog:generated unreleased -->\n<!-- changelog:end -->\n' > CHANGELOG.md
  git add -A && git commit -qm "chore: start" && git tag v0.1.0
  git commit -q --allow-empty -m "fix: a merged fix (#1)"
)
red "a merged fix commit with no entry" "block differs from the commits since" \
  bash -c 'cd "$1" && python3 "$2/changelog.py" --check' _ "$M" "$PWD/$S"
red "a PR whose own entry is missing" "block differs from the commits since" python3 $S/changelog.py --check "${CL[@]}" --with "fix(mcp): a new fix (#999)"
red "a non-conventional commit subject" "not a conventional commit subject" python3 $S/changelog.py --check "${CL[@]}" --with "Update stuff (#999)"
sed 's/^version = "\([0-9.]*\)"/version = "99.0.0"/' Cargo.toml > "$T/Cargo.toml"
red "a release PR without its version block" "changelog:generated v99.0.0" python3 $S/changelog.py --check "${CL[@]}" --cargo "$T/Cargo.toml"
# A BREAKING CHANGE footer marks the entry, in a throwaway repo with one tagged release.
G="$T/repo"; mkdir -p "$G" && (
  cd "$G" && git init -q && git config user.email t@example.invalid && git config user.name t
  printf '[package]\nname = "x"\nversion = "0.1.0"\n' > Cargo.toml
  printf '# Changelog\n\n<!-- changelog:generated unreleased -->\n<!-- changelog:end -->\n' > CHANGELOG.md
  git add -A && git commit -qm "chore: start" && git tag v0.1.0
  git commit -q --allow-empty -m "feat: drop the old flag" -m "BREAKING CHANGE: --old is gone"
  python3 "$OLDPWD/$S/changelog.py" --write >/dev/null
) && if grep -qF -- '- **BREAKING** drop the old flag' "$G/CHANGELOG.md"; then echo "  ok   a BREAKING CHANGE footer marks the entry"
else echo "  FAIL a BREAKING CHANGE footer marks the entry"; fail=1; fi
# #87 item 3: BREAKING CHANGE counts only in the trailing footer block, not in body prose.
P="$T/prose"; mkdir -p "$P" && (
  cd "$P" && git init -q && git config user.email t@example.invalid && git config user.name t
  printf '[package]\nname = "x"\nversion = "0.1.0"\n' > Cargo.toml
  printf '# Changelog\n\n<!-- changelog:generated unreleased -->\n<!-- changelog:end -->\n' > CHANGELOG.md
  git add -A && git commit -qm "chore: start" && git tag v0.1.0
  git commit -q --allow-empty -m "fix: explain the old flag" \
    -m "BREAKING CHANGE: was considered here and rejected." -m "Signed-off-by: t <t@example.invalid>"
  python3 "$OLDPWD/$S/changelog.py" --write >/dev/null
) && if grep -qF -- '- explain the old flag' "$P/CHANGELOG.md" && ! grep -qF 'BREAKING' "$P/CHANGELOG.md"; then
  echo "  ok   BREAKING CHANGE in body prose is not a footer"
else echo "  FAIL BREAKING CHANGE in body prose is not a footer"; fail=1; fi
green "PR title: conventional" python3 $S/pr-title.py "fix(install): say already installed on a re-run"
red "PR title: not conventional" "not a conventional commit subject" python3 $S/pr-title.py "Update install script"
red "PR title: unknown type" "not a conventional commit subject" python3 $S/pr-title.py "feature: new thing"

# (c) Cargo.toml metadata
green "Cargo.toml as committed passes" python3 $S/check-cargo-meta.py
sed 's/^keywords = \[\(.*\)\]/keywords = [\1, "sixth"]/' Cargo.toml > "$T/k.toml"
red "six keywords" "6 keywords" python3 $S/check-cargo-meta.py "$T/k.toml"
sed 's/^categories = \[/categories = ["made-up-category", /' Cargo.toml > "$T/c.toml"
red "unknown category" "'made-up-category' is not a crates.io category slug" python3 $S/check-cargo-meta.py "$T/c.toml"
grep -v '^homepage = ' Cargo.toml > "$T/h.toml"
red "missing homepage" "homepage is missing" python3 $S/check-cargo-meta.py "$T/h.toml"

[ "$fail" = 0 ] && echo "docs-gates red controls: all pass" || echo "docs-gates red controls: FAILED"
exit "$fail"
