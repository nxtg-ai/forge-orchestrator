#!/usr/bin/env bash
# Stamp server.json for a release tag before publishing to the MCP Registry.
#
# The committed server.json is a template: its version, asset URLs and sha256s
# describe the last published release. This script rewrites all three from the
# release itself, so nobody edits server.json by hand at bump time (a hand edit
# that was skipped is why v1.6.1 re-submitted 1.6.0 and the registry refused it).
#
# Usage: stamp-server-json.sh <tag> [server.json]   (needs gh + jq; GH_TOKEN in CI)
set -euo pipefail

TAG="${1:?usage: stamp-server-json.sh <tag> [server.json]}"
FILE="${2:-server.json}"
REPO="${GITHUB_REPOSITORY:-nxtg-ai/forge-orchestrator}"
VERSION="${TAG#v}"

cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ "$cargo_version" != "$VERSION" ]; then
  echo "error: tag $TAG does not match Cargo.toml version $cargo_version" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Hash the exact bytes served at each identifier URL, not a sibling archive.
names=$(jq -r '.packages[].identifier | sub(".*/"; "")' "$FILE")
for name in $names; do
  gh release download "$TAG" -R "$REPO" -p "$name" -D "$tmp" \
    || { echo "error: release $TAG has no asset $name" >&2; exit 1; }
done

stamped=$(jq --arg v "$VERSION" '.version = $v' "$FILE")
for name in $names; do
  sha=$(sha256sum "$tmp/$name" | cut -d' ' -f1)
  url="https://github.com/$REPO/releases/download/$TAG/$name"
  stamped=$(jq --arg n "$name" --arg u "$url" --arg s "$sha" \
    '(.packages[] | select(.identifier | endswith("/" + $n))) |= (.identifier = $u | .fileSha256 = $s)' \
    <<<"$stamped")
done

printf '%s\n' "$stamped" > "$FILE"
echo "stamped $FILE for $TAG:"
jq -r '"  version \(.version)", (.packages[] | "  \(.identifier) \(.fileSha256)")' "$FILE"
