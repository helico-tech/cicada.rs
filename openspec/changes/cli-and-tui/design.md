# Design

## Context

See proposal.md, Why.

- `cicada-client::ActorClient` has a method for every operation in `docs/openapi.json`. Missing items are `None`,
  error responses are `ClientError::Api(Problem)`. `ActorClient::with_client` takes a `reqwest::Client`, and that
  client's default headers apply to every request.
- The view types in `cicada::api` (`ActorView`, `MessageView`, `MessageStatusView`, `OutcomeView`,
  `RunningTurnView`, `NodeView`, `TypeView`, …) are `Serialize`, so JSON and YAML output are just those types.
- Listings are pages (`PageView { items, next }`, limit 1..1000).
- An actor address is `type/key` (`ActorAddress::parse`).

## Goals / Non-Goals

**Goals:**

- One binary for one-off commands and for watching.
- Columns of a resource are defined once and used by both the CLI table and the TUI.
- A slow or unreachable node never freezes the TUI.

**Non-Goals:**

- A client that does not depend on the `cicada` crates.
- Any change to the server.

## Decisions

### 1. One crate, one binary, `cicada`

`crates/cicada-cli` builds the binary `cicada`, with `cicada tui` for the terminal UI. Config, client setup and
columns are shared, and there is one thing to install. Two crates would need a third, shared one for that code.

The crate has a small library (`lib.rs`) so that integration tests can call `run(cli, &mut out)` with parsed
arguments and an output buffer; `main.rs` only parses arguments, calls `run` and prints the error.

### 2. Commands

Global flags: `--context`, `--server`, `--token`, `-o/--output table|wide|json|yaml`.

| Command | Calls |
| --- | --- |
| `get actors [--prefix P] [--limit N] [--all]` | `list_actors` |
| `get actor ADDR` | `get_actor` |
| `get mailbox ADDR` | `mailbox` |
| `get messages [--prefix P]` | `list_pending` |
| `get message ID` | `status` |
| `get caused ID` | `caused_by` |
| `get outcomes [--status S] [--prefix P] [--since T]` | `outcomes` |
| `get turns [--node N] [--prefix P]` | `turns` |
| `get turn ADDR` | `turn` |
| `get nodes` / `get node ID` | `nodes` / `node` |
| `get types` / `get type NAME` | `types` / `get_type` |
| `describe actor\|message\|node\|type X` | the single `get`, always shown as YAML |
| `create actor ADDR [--state JSON]` | `create_actor` |
| `delete actor ADDR` / `delete type NAME` | `delete_actor` / `remove_type` |
| `send ADDR --type T [--payload JSON] [--id ID] [--delay PT5S \| --at INSTANT] [--correlation-id C] [--header K=V]` | `send_message` |
| `cancel message ID` / `cancel turn ADDR` | `cancel` / `cancel_turn` |
| `cancel messages (--id ID… \| --prefix P [--type T])` | `bulk_cancel` |
| `retry message ID` / `retry messages (…same…)` | `retry` / `bulk_retry` |
| `deliver-now ID` | `deliver_now` |
| `pause ADDR` / `resume ADDR` / `purge ADDR` | `pause_mailbox` / `resume_mailbox` / `purge_mailbox` |
| `stats [--prefix P]` | `stats` |
| `health` | `health` |
| `config get-contexts` / `config current-context` / `config use-context NAME` / `config set-context NAME --server URL [--token T] [--header K=V]` | the config file |
| `tui` | the terminal UI |

A listing shows one page of `--limit` (default 100); `--all` follows `next` until the last page. A missing item is an
error with exit code 1 and a message such as `actor account/acme:7 not found`. `ClientError::Api` prints the
problem's title and detail.

### 3. Configuration

```toml
current-context = "local"

[contexts.local]
server = "http://localhost:8080"

[contexts.prod]
server = "https://cicada.internal/api"
token = "…"                       # sent as `Authorization: Bearer …`
headers = { "X-Tenant" = "ops" }
```

Path: `$CICADA_CONFIG`, else `$XDG_CONFIG_HOME/cicada/config.toml`, else `$HOME/.config/cicada/config.toml`. The
same path on macOS as on Linux, which is what kubectl users expect. No `dirs` crate.

Resolving the target:

1. The server is `--server`, else `CICADA_SERVER`, else the context's `server`.
2. The context is `--context`, else `CICADA_CONTEXT`, else `current-context`. A named context that does not exist is
   an error. Without a server from steps 1 and no context, the command fails and says how to set one.
3. The token is `--token`, else `CICADA_TOKEN`, else the context's `token`. Headers come only from the context.

A missing config file is the same as an empty one. Resolution is a pure function of the flags, an environment map and
the parsed file, so it is tested without touching the real environment. `config set-context` and `use-context` rewrite
the whole file (`toml::to_string_pretty`); comments in it are not kept, and the command's help says so.

### 4. Output: one `Columns` trait

```rust
pub trait Columns {
    fn headers(wide: bool) -> Vec<&'static str>;
    fn cells(&self, wide: bool) -> Vec<String>;
}
```

It is implemented for each view type in `output.rs`. `print_table` aligns columns with plain padding; the TUI builds
`ratatui::widgets::Row`s from the same cells. `-o json` is `serde_json::to_string_pretty` of the view (a listing prints
the items as an array), `-o yaml` is `serde_norway`. Instants are shown as an age (`5s`, `3m`, `2h`, `4d`) in a table
and as full ISO-8601 in JSON and YAML, and in wide output.

Other option: one table-building library (`comfy-table`, `tabled`). A dozen lines of padding do the job, and the TUI
cannot use those libraries anyway. Not chosen.

### 5. TUI: k9s-style, state without I/O, a loop that never waits on HTTP

Layout: a header (context, server, node health, actors / pending / running turns, oldest due message; the key hints of
the current screen in columns; the logo), an optional prompt box for `:` and `/`, the screen, and a bottom line with
breadcrumbs and a toast that fades. One theme of the 16 ANSI colours, so it reads on light and dark terminals: status
words carry a colour (`pending` yellow, `running` cyan, `done` green, `failed` red, `cancelled` grey, `purged`
magenta; nodes `alive` / `suspect` / `dead`), failed rows are red and finished business is grey.

Screens form a stack: Enter drills down, Esc goes back (after clearing a filter), a `:` command replaces the stack.

- Listings: `:actors`, `:messages`, `:outcomes [status]`, `:turns`, `:nodes`, `:types`, each with an optional address
  prefix (`:actors e2e.`); `:mailbox ADDR`.
- Overviews: actor (facts, state JSON, tabs mailbox / outcomes), message (facts, payload, the messages it caused; `a`
  its actor, `b` its cause), node (facts, its turns), type (facts, schemas; the pane shows the selected schema). Also
  `:actor`, `:message`, `:node`, `:type` to jump to one.
- Describe (`d`): the item as coloured YAML or JSON (`t` toggles, `w` wraps).

Tables: `/` filters the shown rows as you type (the visible cells, not hidden sort keys); `<` `>` pick the sort column
and `s` flips it, a click on a header sorts by it; `w` shows the wide columns. A refresh keeps filter, sort and the
selected row. Actions follow k9s: `ctrl-d` delete, `ctrl-k` stop the running turn, and `c` cancel, `r` retry, `n`
deliver now, `p` / `u` pause / resume, `x` purge, each behind a dialog whose default is No (red for destructive
ones). An action applies to the selected row, else to the screen's subject; on a message screen message actions apply
to that message. `:` has Tab completion of commands and outcome statuses. Mouse: wheel, click to select, click the
selected row to open.

Code: `app.rs` (state, input, effects; no I/O), `grid.rs` (rows, filter, sort, selection), `fetch.rs` (API calls),
`theme.rs`, `highlight.rs` (JSON / YAML colouring), `ui.rs`, and `mod.rs`: the loop, a `tokio::select!` over input, a
250 ms tick and a channel of results from spawned requests. Each screen fetch carries a sequence number, so an answer
for a screen the user has left is dropped, and no second fetch starts while one runs. The screen refreshes every 2 s;
the header every 10 s, because `GET /stats` reads every row. The terminal (and mouse capture) is restored on exit,
error and panic.

Other option: fetch inside the draw loop. Simpler, but the screen hangs on every slow request, and a slow or dead node
is the moment someone opens the TUI. Not chosen.

## Risks / Trade-offs

- **Outcomes are listed oldest first by the API, at most 1000.** The TUI shows them newest first, but on a busy node
  they are the newest of the 1000 oldest. Fixing that needs an API change (a separate proposal).
- **`get actors` shows only addresses.** `GET /actors` returns addresses; one `GET /actors/{a}` per row would be N
  requests. The table has the address, type and key; `describe actor` has the rest.
- **Config rewrite loses comments.** Accepted for a tool-managed file; documented in the help.
- **A new UI dependency tree (`ratatui`, `crossterm`).** Only the CLI crate pulls it in; the library crates do not
  change.

## Testing

- `config.rs`: unit tests of resolution (flag over env over file, unknown context, no server), of the path rules and
  of a round trip of the file.
- `output.rs`: unit tests of the age format and of a table with fixed views.
- `tests/cli.rs`: `run` against a real `actor_api()` node on a local port, as in `crates/cicada-client/tests`:
  create, get, describe, send, cancel, retry, pause/resume, purge, delete, bulk, outcomes, nodes, types, stats, each
  output format, a missing item and an API error.
- `tui/app.rs`, `tui/grid.rs`, `tui/theme.rs`, `tui/highlight.rs`: unit tests of commands and completion, the live
  filter, the confirmation (No by default), which action a key means on a row or a subject, drill-down and back,
  dropping late answers, the tick schedule, the mouse, sorting and the colours. `TestBackend` renders of a listing
  (header, logo, rows, crumbs), the prompt, the dialogs and an overview.
