# Design

## A consumer skill, backed by the project's source

Maintain skills/cicada/SKILL.md and scripts/cicada_source.py beneath it. Frontmatter has name `cicada`, a precise
description for building/testing/operating Rust projects with Cicada, and metadata.cicada-version equal to the
workspace version. No harness metadata, connector, contributor workflow or automatic installation. The owner asked
to design this change now; its queue position remains after listener-probe-on-close.

Keep the entry short (roughly 120–180 lines): resolve source, essential practices, a task-to-source routing table,
verification and update behavior. All detailed knowledge lives in the existing guides, examples and API source.
Cargo git checkouts currently contain that tree; path dependencies point directly at it. Ship no mirrored manuals:
they add generation/link rewriting and still cannot substitute for the consumer's older or modified dependency.

Routes use repository-relative paths in code spans, optionally with headings, explicitly resolved beneath the helper's
reported source root, never beneath the installation folder. Do not use ../../docs links that break when installed.
Include the README, architecture, all guides, operations, generated HTTP/CLI/contracts references and worked example
modules, grouped by task. The checker validates these routes in this repo; consumers resolve them in their own source.

## Read-only source discovery

The Python 3.11+ standard-library helper runs from the consumer cwd, with optional --manifest-path and --target. It obtains
the host triple from rustc -vV when target is omitted, then runs cargo metadata --format-version 1 --locked --offline
--filter-platform TARGET with the selected manifest. Use argument arrays, no shell, and RUSTUP_AUTO_INSTALL=0 for
Cargo/rustc subprocesses; offline alone does not constrain rustup proxies. Bound subprocesses and report missing tools/
toolchains or errors, never install them or retry without locked/offline. Require an existing lockfile; do not resolve/
write one. Forward --no-default-features, --all-features and repeatable --features when explicitly selected; report
the chosen target/features so the result does not imply all build configurations were examined.

Match the seven package names cicada, cicada-test, cicada-axum, cicada-client, cicada-otel, cicada-actors, cicada-app,
not a prefix that also matches a consumer such as cicada-code-factory. Select package IDs from the filtered resolve
graph before grouping their packages records: excluded target packages may remain in that array. No
metadata --no-deps, cargo tree hash guessing or dependency-version inference from a manifest range. Resolve source
roots from manifest paths: for the current layout, parents[2], verifying crates/cicada/Cargo.toml and docs there.
Report each package id/version/source/manifest and candidate root; a missing docs tree never hides a found dependency.

Emit one JSON report: status, packages, groups, target, feature selection, workspace, skill_version and skill_source,
with each group recording version, source identity, root, commit, dirty and docs_available. Status is ok, none, mixed,
self, no-docs or unavailable. Group git packages by full Cargo source identity and version, path packages by canonical
Cicada root and version; missing roots retain distinct manifest provenance rather than silently grouping unrelated
paths. Any differing version/source group is mixed, including same-version forks and patches. For git, use Cargo's
full revision and tag/branch/rev selector; probe the actual root for HEAD/dirty too and expose discrepancies. For path,
report actual HEAD/dirty or unversioned. Do not compare Git commits by age or call a fork an upstream release.

self requires the actual core package to be a member of the selected workspace, with the verified Cicada tree at
that workspace root, not merely a matching consumer name. Route such requests to the repo's own instructions, without
imposing them on consumers. --metadata FILE supports fixture input; test roots are supplied as temporary trees.
Unavailable includes a concise diagnostic and reason (missing-lock, tool/metadata failure or timeout); do not depend
on fragile parsing of Cargo's English errors. A missing download may suggest cargo fetch --locked, never run it.

Per group, version_relation says project older/equal/newer/unknown relative to skill_version; revision_match is
same/different/unknown against the skill archive's SOURCE.json commit. Equal versions alone do not establish a match.
A directly copied skill with no SOURCE.json has unknown revision; a dirty source remains explicitly modified even
with matching HEAD. No version substitution, dependency upgrade, lockfile write or installed-skill overwrite occurs.

## Using the result

Read only the task's routes from the resolved source; its architecture and code govern that dependency. On a missing
file/heading, search that source's README, docs and public code: headings may have moved. Say unsupported only when
evidence supports it, otherwise unverified. Never fill gaps from latest main or a proposal. For no-docs, inspect the
exact package's source/rustdoc; cargo doc/check/test may be run as part of the user's authorized work, with normal
build effects, not as a purported read-only helper action. Pinned upstream docs are a fallback only after provenance
is established; a fork/path modification must not be silently replaced by upstream content.

On mixed, show groups and establish the intended package/configuration before advice that assumes one source;
unrelated inspection can continue. On unavailable or missing Python, use lockfile and manifests for partial identity,
then verified local source paths where available; state the uncertainty rather than guessing a cache directory.
Do not stop all useful work simply because docs/network are absent. The helper never fetches or changes credentials.

For a new project, prefer one verified published stable release for all selected Cicada crates, with explicit features
for the selected layer/store. Tags alone do not prove a successful release (ADR 0016). Use a known published skill
release as a candidate offline and disclose that latest is unverified. Preserve explicit user choices of a commit,
fork or path; never prohibit them or silently replace them. Adding/fetching dependencies follows the user's task,
not an unconditional rule that requires a second permission for already requested integration.

## Knowledge and maintenance

The entry reminds agents of consistency boundaries/stable actor identity, atomic internal effects versus at-least-once
external work and stable idempotency keys, sender-selected retry versus lost-claim limits, cooperative cancellation,
contract checks and virtual-time tests. It routes rather than restates APIs. Durable waits use scheduled messages or
flow sleep; bounded async work within a turn is not universally forbidden. Blocking tasks can outlive cancellation.
Flow replay/step ids/step keys, ready-made actors, host HTTP authentication, Worker uncertainty, telemetry and operations
are loaded when relevant and supported by the resolved version. Do not mandate extra crates or features unnecessarily.

Add a core-guide section on designing actor types: ownership/serialization boundaries, key/cardinality choices,
bounded state/work and messages, state evolution and external effects. Use current checked examples. serde defaults
help read old values but do not waive strict contract-hash equality; rolling deployment requires compatible contracts
and migration rules of that release. Never prescribe new-before-old for every migration or auto-approve a destructive
contract change. Missing reusable knowledge is added to its guide, not a parallel best-practices manual in the skill.

## Release artifact and verification

Add scripts/package-skill.py TAG OUTPUT. Resolve the tag's commit, read version and skill files from its Git tree and
create cicada-skill-vX.Y.Z.tar.gz containing cicada/SKILL.md, cicada/scripts/cicada_source.py and cicada/SOURCE.json
(repository, tag, commit, version). Validate matching versions/tag, expected paths and portable archive members; no
working-tree files, timestamps or absolute paths enter the content. Use standard-library tar/gzip and git reads;
repeat packaging of one commit produces the same archive. No global clean-tree requirement or copied dependencies.

CI tests use a temporary Git repo with the skill tree and test tag, not an old release that predates the skill. The
release job creates the archive after existing checks and supplies it to gh release create; a packaging failure must
prevent release creation. Releasing includes the skill version and archive checks. ADR 0041 records this release
convention; no runtime guarantee or wire change. README documents extracting into a staging folder, then installing
in the host agent's project skill directory, and diffing before replacing a customized copy. Prior releases have no
asset: use the oldest/newer available router with the older dependency's sources and disclose the mismatch.

Tests cover fixtures, source failures without writes/network fetch, route existence/anchors and domain coverage,
frontmatter/version, release layout/provenance/reproducibility and existing compiled guide examples. Maintain scenario
evals for realistic consumer tasks; an independent review exercises a representative subset in temporary projects.
No mandatory model service in CI, brittle exact-response assertions or claim that checks prove complete knowledge.
Five tasks: guide practices, skill/resolver, checks/evals, release packaging, docs/ADR; then finishing checks.
