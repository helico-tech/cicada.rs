# Docs handoff for node-details

Version: 0.19.0 (breaking: public structs gain fields). Refresh the dependency pins of `docs/how-to/install.md` and the
README release pins once it is published.

## What changed

Each node row carries three optional, display-only details (ADR 0048): the OS host name, the version of the `cicada`
crate and a configured label.

- `NodeConfig::label: Option<String>`, default `None`. A present label is not blank after trimming, has no NUL and at
  most 200 characters (`limits::MAX_ID_LENGTH`); it is kept as given, outer spaces included. A bad one is a
  `StartProblem` at `initialize` and `start` (`label must not be blank`, `label must not contain NUL`, `label must be
  at most 200 characters long`). Tests: `tests::runtime::startup::a_blank_a_nul_or_a_too_long_label_fails_start`,
  `no_label_or_one_of_200_characters_with_outer_spaces_is_valid`.
- `start()` looks the host name up once (whoami 2) after initialization. A failed lookup, non-UTF-8, blank, NUL or
  over 200 characters gives no host name and one debug log line without the value; the node starts anyway. The value is
  not trimmed or truncated and is not refreshed while the node runs. `cicada_version` is the core crate's version, not
  the application's. Tests: `runtime::system::tests::*`,
  `tests::runtime::operator::a_started_node_reports_its_version_and_label_and_an_initialized_one_has_no_row`,
  `a_send_only_node_without_a_label_has_its_version_and_no_label`.
- Every heartbeat replaces all three fields, absent ones included. `initialize`/`run_due_turns` alone and a worker over
  HTTP add no row; started send-only systems and workers on the store do. Lifecycle, health thresholds, expiry and
  clean-stop removal are unchanged.
- `NodeInfo` and `NodeView` gain `hostname`, `cicada_version`, `label`. `GET /nodes` and `GET /nodes/{node}` serve
  `hostname`, `cicadaVersion`, `label`, left out when absent; a new client reads missing or `null` as `None`. No route
  or client method is added. Tests: `cicada-axum` `operator::both_node_routes_carry_the_details_and_leave_out_absent_ones`,
  `cicada-client` `client::node_details_are_read_when_present_and_none_when_absent_or_null`.
- Schema version 7 on SQLite and PostgreSQL: nullable `nodes.hostname`, `nodes.cicada_version`, `nodes.label`.
  Additive: running 0.18 nodes keep using their prepared statements (their inserts have no details, their updates
  leave another writer's details in place). A 0.18 binary refuses the migrated store, so old nodes cannot restart or
  roll back. Tests: `store::{sqlite,postgres}::upgrade_tests::a_database_of_schema_version_6_keeps_its_nodes_and_its_running_old_nodes`,
  conformance `nodes_a_heartbeat_replaces_the_details_unicode_included_and_absent_ones_clear_them`.
- `cicada-app`: `--node-label` / `CICADA_NODE_LABEL` for `serve` and a worker on the store; precedence flag, variable,
  host `NodeConfig.label`, none. The variable is read raw: empty is unset, blank is invalid, outer spaces are kept. A
  bad flag or variable is a usage error (exit 2); a bad host label fails the start (exit 1). A worker over HTTP refuses
  the flag (exit 2, "a worker over HTTP does not take --node-label: it has no row in the node table") and reads neither
  the variable nor the host label. `docs/reference/cli.md` is regenerated.
- Node tables: `ID HOSTNAME STATUS TURNS LAST-SEEN AGE`; `-o wide` adds `HOSTED VERSION LABEL`; the TUI nodes screen
  uses the same columns. Missing values show `-`. Table/TUI cells escape `\n`, `\r`, `\t` and other control characters
  as `\u{hex}`; JSON/YAML keep the text.

## 1. Node table

**Where.** `docs/operations.md`, `Node table`.
**Text.** Rows also show the host name the node looked up at start, the version of the `cicada` crate it was built
with, and its label (`NodeConfig::label`, `--node-label`, `CICADA_NODE_LABEL`), for example the application and its
version. `cicada get nodes` shows `HOSTNAME`; `-o wide` adds `HOSTED`, `VERSION` and `LABEL`; JSON/YAML show
`hostname`, `cicadaVersion` and `label` when present. A container's host name may change at each start. Ids stay random
by default: set `CICADA_NODE_ID` where an id should survive a restart (for instance from a StatefulSet pod name), and
keep it unique among live nodes.
**Caveats.** Display only, like the rest of the table. Nodes that share an id share one row: the last writer's details
win, and either node's clean stop removes it. A 0.18 node writing during the rollout can leave stale details. Neither
the host name nor the label is a unique or authenticated identity; the host name can identify a machine, and the API's
access control covers it.

## 2. App settings

**Where.** `docs/guide/app.md`, `Settings` (node table of flags) and `A worker over HTTP`.
**Text.** Add the row `--node-label` | `CICADA_NODE_LABEL` | `label`. Note that this variable is read as given: empty is
unset, whitespace only is an error, outer spaces are kept. Under `A worker over HTTP`: it has no node row, so
`--node-label` is refused and `CICADA_NODE_LABEL` and the host's label are ignored.

## 3. Core configuration

**Where.** `docs/guide/core.md`, `Configuration` (optional).
**Text.** Row `label` | none | "Free text shown with the node in the node table; not blank, no NUL, at most 200
characters; display only."

## 4. Architecture

**Where.** `docs/architecture.md`, the schema paragraph under `Store requirements`.
**Text.** The schema is at version 7. Migration 7 adds the nullable columns `nodes.hostname`, `nodes.cicada_version`
and `nodes.label` (ADR 0048); existing rows have none. It is additive: a running node of version 6 keeps working, but a
node of version 6 refuses to open the migrated database.

## 5. Upgrading

**Where.** `docs/how-to/upgrading.md`, the intro ("Stores write schema version 6") and a new section for 0.19.0.
**Text.** Stores write schema version 7. 0.19.0: `NodeConfig`, `NodeInfo` and `NodeView` gain public fields, so struct
literals without `..Default::default()` and exhaustive destructuring stop compiling. Migration 7 is additive: running
0.18 nodes keep working while it is applied, but once a 0.19 node opened the store a 0.18 node cannot start on it, so
neither restart nor roll back old nodes; replace them during the rollout.

## Blocking

None. Every doctest, link, guarantee row and skill route passes without prose edits; no guide route, heading or G/S row
is needed.

## Status

Proposed text only; no owner fixes were needed or integrated. The host name lookup ran in tests on macOS (Darwin) and runs in
the Linux E2E containers, where no test asserts its value; other platforms are untested, and a failed lookup never
stops a start.
Check results are in the build report.
