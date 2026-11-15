#!/usr/bin/env bash
# Checks that a version can be released and prints its changelog section, which becomes the release notes.
#
#   scripts/release-check.sh [--before-push] vX.Y.Z
#
# Run it on the commit to release: the checked-out commit is the one checked, and a local tag of that name must point
# at it. It fails, naming the problem, when the tag is malformed or points elsewhere, the workspace version or the
# version of the consumer skill (skills/cicada/SKILL.md) differs, the changelog has no dated section for the version,
# or, without --before-push, the commit is not on origin/main after a fetch. scripts/gate.py release runs it with
# --before-push before main is pushed and without it afterwards.
# CHANGELOG_FILE reads another changelog than CHANGELOG.md.
set -euo pipefail

fail() {
    echo "release-check: $1" >&2
    exit 1
}

before_push=false
if [ "${1:-}" = "--before-push" ]; then
    before_push=true
    shift
fi
[ $# -eq 1 ] || fail "usage: scripts/release-check.sh [--before-push] vX.Y.Z"
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

# The checked-out commit; a tag that already exists must point at it, not at another commit.
commit=$(git rev-parse HEAD)
tagged=$(git rev-parse --quiet --verify "refs/tags/$tag^{commit}" || true)
[ -z "$tagged" ] || [ "$tagged" = "$commit" ] || fail "tag $tag points at $tagged, not at the checked-out $commit"
if ! $before_push; then
    git fetch --quiet origin main
    git merge-base --is-ancestor "$commit" origin/main || fail "commit $commit is not on main"
fi

awk -v heading="$heading" '
    $0 == heading { found = 1; next }
    found && /^## / { exit }
    found { print }
' "$changelog"
