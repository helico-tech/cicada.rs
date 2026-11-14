#!/usr/bin/env bash
# Checks that a version can be released and prints its changelog section, which becomes the release notes.
#
#   scripts/release-check.sh vX.Y.Z
#
# Run it on the commit to release. It fails, naming the problem, when the tag is malformed, the workspace version or
# the version of the consumer skill (skills/cicada/SKILL.md) differs, the changelog has no dated section for the
# version, or the commit is not on origin/main.
# CHANGELOG_FILE reads another changelog than CHANGELOG.md.
set -euo pipefail

fail() {
    echo "release-check: $1" >&2
    exit 1
}

[ $# -eq 1 ] || fail "usage: scripts/release-check.sh vX.Y.Z"
tag=$1
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "'$tag' is not a version tag such as v1.2.3"
version=${tag#v}

# Every crate of the workspace, so also one that sets its own version.
versions=$(cargo metadata --no-deps --format-version 1 | jq -r '[.packages[].version] | unique | join(" ")')
[ "$versions" = "$version" ] || fail "the tag is $tag, but the workspace version is $versions"

skill=$(sed -n 's/^  cicada-version: *//p' skills/cicada/SKILL.md | head -n 1)
[ "$skill" = "$version" ] || fail "the tag is $tag, but skills/cicada/SKILL.md says cicada-version ${skill:-nothing}"

changelog=${CHANGELOG_FILE:-CHANGELOG.md}
heading=$(grep -E "^## ${version//./\\.}( |$)" "$changelog" | head -n 1 || true)
[ -n "$heading" ] || fail "$changelog has no section for $version"
[[ $heading =~ ^##\ [0-9.]+\ —\ [0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] ||
    fail "the section of $version must be headed '## $version — YYYY-MM-DD', was '$heading'"

# The commit of the tag once it exists, the current one before tagging.
commit=$(git rev-parse --quiet --verify "refs/tags/$tag^{commit}" || git rev-parse HEAD)
git fetch --quiet origin main
git merge-base --is-ancestor "$commit" origin/main || fail "commit $commit is not on main"

awk -v heading="$heading" '
    $0 == heading { found = 1; next }
    found && /^## / { exit }
    found { print }
' "$changelog"
