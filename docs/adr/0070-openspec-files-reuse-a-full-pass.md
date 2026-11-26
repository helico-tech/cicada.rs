# 0070. Files under openspec reuse a full pass

Date: 2026-11-26

## Status

Accepted for gate-openspec-reuse. Partly supersedes ADR 0046: the files that may reuse a full pass.

## Context

ADR 0046 lets a commit reuse the nearest full pass when only regular Markdown files changed since. Every landing
archives its change, which moves `.openspec.yaml` and review markers into `openspec/changes/archive`, so the commit
after
each merge needs a second full run, 10 to 20 minutes under load. No build step, test or doctest reads non-Markdown files
under `openspec/`.

## Decision

- Any non-executable regular file under `openspec/` may reuse a full pass, as Markdown under the allowed folders does.
  Symlinks, mode changes and executable files there still need a full run.
- `scripts/review.py` writes its markers as plain files.

## Consequences

An archive commit reuses the full pass of the merged change and runs only the docs checks. A future test or build step
that reads non-Markdown files under `openspec/` must move them out of this rule.
