# Design

## Scope and public model

1. Add optional flat `hostname`, `cicada_version` and `label` strings to private NodeRecord and public NodeInfo/NodeView.
   They describe the writer of a node row; they are not identity, authentication or a placement selector. Keep the
   random node-id default and existing duplicate-id behavior. No pid, HTTP address, arbitrary labels, worker registry,
   hostname-derived id helper or hostname override: each adds policy beyond the operator's display need.
2. NodeConfig gains `label: Option<String>`, default None. A present label must be nonblank after trimming, at most
   `limits::MAX_ID_LENGTH` (200) Unicode scalar values, and NUL-free. Preserve the supplied string, including outer
   spaces; trimming tests blankness only. NodeConfig validation reports a StartProblem at all existing validation
   sites. Do not change NodeId validation. Backend details are trusted internal input, not a second validation API.
3. Resolve hostname once on successful start, after initialization and before spawning background work, using a direct
   `whoami = "2"` dependency (2.1.3 already locked). No environment-variable fallback or subprocess. An error,
   non-UTF-8 result, blank, NUL or overlong value gives None and one debug diagnostic without the rejected value.
   Use the same 200-character bound; do not truncate or refresh it at each heartbeat. A small pure result-normalizing
   helper makes failure cases deterministic to test, without a public override or environment mutation.
   `cicada_version` is Some of the core crate's `env!("CARGO_PKG_VERSION")`, not the embedding application's version.

## Heartbeats and migration

4. Carry the captured fields in a private NodeDetails value through `heartbeat_node(id, hosted, details)` on Backend
   and its handle. Every new-code heartbeat replaces all three fields, including None, in memory and both SQL stores.
   Existing timestamps, ten-second cadence, health thresholds, clean-stop removal and one-day expiry stay unchanged.
   `start()` still spawns the initial heartbeat; it does not synchronously promise that a row already exists on return.
   Initialization/run_due_turns alone add no row. Send-only started systems do; remote Worker does not.
5. Append migration 7 to each SQL backend: nullable TEXT columns hostname, cicada_version and label on nodes.
   No old migration changes, table rewrite or new index. Existing rows read None. Name every selected/inserted column,
   and replace metadata in the new upsert while preserving started_at. Memory mirrors these semantics. Update
   existing runtime/fault/fuzz wrappers and row constructors when the private signature changes.
6. Old running schema-6 connections can continue with their four-column insert/upsert and explicit projections.
   An old insert leaves metadata NULL; its update leaves existing metadata untouched, so an id reused across versions
   may show stale details. A new heartbeat replaces them. Old binaries reopening a migrated database are refused by
   the existing version guard; they cannot restart or roll back after migration. Tests must prove migration and the
   old prepared statements, not merely inspect SQL. No stop-the-world upgrade is needed for these additive columns.
7. Duplicate ids still share a row and clean stop may remove another process's row. New-code metadata is last-writer
   wins; mixed-version rows may be stale. Hostnames can also change legitimately. Neither a changing hostname nor
   these fields proves a collision. The successor pinned-actors must settle placement/identity safety independently.

## HTTP and App

8. NodeView serializes `hostname`, `cicadaVersion`, `label`, each with serde default and omission when None.
   NodeDetailView inherits them through its existing flatten. New clients accept old absent fields (and null as None);
   old tolerant clients ignore new fields. Regenerate OpenAPI; no endpoint or client method is added. Existing host
   authentication controls access. Describe hostname as potentially identifying a machine/container, not as a secret
   or as an authenticated fact. No new auth flow.
9. `--node-label` / `CICADA_NODE_LABEL` apply to serve and direct-store workers: flag > nonempty variable > host
   NodeConfig.label > None. Use normal App parsing errors (exit 2) for invalid explicit values; invalid host config
   follows start validation (exit 1). Empty environment values are unset, whitespace-only values are invalid.
   Read this variable raw: existing `variable()` trims and treats whitespace-only as unset, so it cannot be reused.
   HTTP worker refuses an explicit flag with a no-node-row diagnostic and ignores both the variable and host label,
   including malformed values. WorkerConfig and lease owner fields do not change.
10. Standard node tables add HOSTNAME after ID; wide tables keep HOSTED and add VERSION and LABEL. Missing values
    use the existing `-` convention. Escape control characters in these metadata cells: newline/carriage return/tab
    as `\n`/`\r`/`\t`, other Unicode control characters as `\u{hex}`; preserve printable Unicode. Stored values and
    JSON/YAML are unchanged. TUI's shared Columns/Grid path gains the same fields, with wide details available;
    get/describe JSON/YAML follow the optional wire view. Test the shared table and a TUI node view, and regenerate CLI.

## Compatibility, proof and docs ownership

11. ADR 0048 supplements 0015. Public NodeConfig/NodeInfo/NodeView field additions break exhaustive struct
    construction/destructuring outside the repo. Do not add non_exhaustive, which would also prohibit current config
    update syntax. Target 0.19.0 (next breaking minor if another release has advanced main); schema is 7 unless a
    predecessor appends a migration, which must be reconciled before building. Existing in-repo config literals use
    defaults and no protected doctest is known to break. Existing actor contracts and G/S guarantees do not change.
12. Conformance checks details, Unicode and overwrite-to-None on all backends. SQL upgrade tests preserve old rows,
    exercise a live pre-migration connection's prepared node read/heartbeat after migration, and prove the old shape
    can insert/upsert with the new columns. Runtime tests use virtual time and await the initial heartbeat; normalization
    unit tests cover hostname errors rather than assuming the test machine's name. HTTP/client tests cover both shapes;
    App tests cover precedence, errors, remote-mode ignoring and table/TUI rendering. No production test hook is needed.
13. Builder owns rustdoc, ADR/index, migrations, generated OpenAPI/CLI, version/changelog and docs-handoff.md.
    Handoff operations' Node table (fields and stable unique id guidance), app's settings/remote-worker rules,
    architecture's node/schema sections, upgrading's public-struct and no-restart/rollback warning, optional core
    configuration guidance and release pins. No new guide/example/heading route or guarantee row is required.
    Protected prose is not edited by the builder; report any unforeseen docs check as blocked. Finish on the full
    committed-tree gate, including PostgreSQL and Docker. OS lookup behavior on untested targets remains a stated
    limitation; its failure never prevents node startup.
