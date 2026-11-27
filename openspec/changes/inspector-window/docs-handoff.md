# Docs handoff for inspector-window

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A new crate `cicada-inspector` (ADR 0076, which supersedes in part ADR 0011 for a native macOS console only; the
browser console stays removed, and ADR 0056's container exception stands). No library crate depends on it; the gate's
dependency check names the one allowed edge, `cicada-tray`'s future optional feature `inspector`
(`inspector-in-tray`).

`cicada_inspector::Inspector::new().run()` opens a native window on macOS 14 or later and owns the main thread until
the window closes or the app quits. It is a regular app with a Dock icon and a main menu: Quit (⌘Q), Close (⌘W),
Refresh (⌘R), Back (⌘[) and Forward (⌘]). On another platform, or before macOS 14, `run` prints "the Cicada
inspector requires macOS 14 or later" and returns exit code 1.

The window: a see-through sidebar (Overview; Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types,
Watches), a toolbar with back and forward, the title (the selected item, with the context as subtitle), a context
pop-up and Refresh. Only the Overview has a page; the other items say "Coming soon". The Overview shows four tiles
(actors, pending messages, oldest due, failed — the failed count is labelled "kept by retention", as the API does not
report the retention; an older server without the count shows `—`), the health, the nodes, the first 20 running turns
(with a line when there are more) and the registered types with their actor counts.

Refresh is by hand (Refresh or ⌘R; live updates come with `inspector-live`). A refresh asks `/stats`, `/health`,
`/nodes`, the first page of `/turns` and `/types` at once, and, once `/types` answers, `/stats?prefix=<type>/` for at
most 50 types, 8 at a time, all within 10 seconds. A part whose call fails shows its error while the others show their
numbers; a part with no answer by the deadline says "no answer within 10 s"; a type past the 50, or not counted by the
deadline, shows "not counted", which differs from a count that failed ("error", with the error as its tooltip). A new
refresh or a context switch cancels the one before, and a result that still arrives is dropped.

The context: the window starts on the context `cicada` would use and the pop-up lists the config file's contexts,
the one in use marked. Picking one makes a new client and refreshes. The order is that of `cicada`: `CICADA_SERVER`
and `CICADA_TOKEN` win over the picked context, then the picked context, else `CICADA_CONTEXT`, else the file's
`current-context`. When either variable is set, the pop-up, the subtitle and the Overview say
"CICADA_SERVER overrides the context" (or both variables). A context without a usable server shows its error in the
window, which stays usable; another context can be picked. Error texts hide the password of a URL
(`scheme://user:***@host`); the client sends the URL's credentials as a header, so transport errors name the URL
without them, and a malformed server URL is refused without quoting it.

New public items in `cicada-app`: `context_client(&Env, Option<&str>) -> anyhow::Result<(ActorClient,
Option<String>)>` (the client, with the operator commands' 30 s timeout, and the context in use) and
`hide_passwords(&str) -> String` (the usage errors' password hiding).

Tests: `crates/cicada-inspector/src/tests.rs` (the Overview against an in-process server, one failing route, an older
server, 60 types with at most 8 counts at a time, the deadline, a refresh discarded after a context switch, a target
error and an override, navigation); `crates/cicada-app/src/config.rs` (`a_context_client_resolves_as_the_operator_commands_and_names_its_context`,
`a_context_client_without_a_server_says_so_without_the_token`,
`a_context_client_refuses_a_malformed_server_without_quoting_it`,
`a_transport_error_of_a_context_client_hides_the_credentials`). The window has a manual check list in `tasks.md`.

No guarantee row, wire format, schema, actor contract or command line changes. The CLI reference and the OpenAPI
document are unchanged.

## 1. The Inspector section of the app guide

**Where.** `docs/guide/app.md`, a new section `Inspector on macOS` after `Menu bar app on macOS`.
**Text.** A host makes a small binary (or a `.app`) that opens the inspector:

```rust
fn main() -> std::process::ExitCode {
    cicada_inspector::Inspector::new().run()   // standalone, on the current context of `cicada config`
}
```

(the crate's doc example, `crates/cicada-inspector/src/lib.rs`; mark it `no_run` if it becomes a doctest of
`AppGuideDoctests`, which needs `cicada-inspector` as a dependency of `cicada-examples`.) Then describe: the window
and its sidebar, the Overview's tiles and lists, Refresh (⌘R) by hand, back and forward (⌘[ and ⌘]), the context
pop-up shared with `cicada config`, and that the other sidebar items come in later versions.
**Caveats.** macOS 14 or later; elsewhere `run` exits with code 1. Run it on the main thread; it owns the app (a
Dock icon, its own main menu) until the window closes, so it cannot be opened inside another app's run loop yet (the
tray's "Open Inspector…" is `inspector-in-tray`). `CICADA_SERVER` and `CICADA_TOKEN` in its environment win over the
picked context, as for `cicada`; the window says so. The failed tile counts what `GET /outcomes` still lists, i.e.
what retention keeps; the inspector cannot show the retention itself. A refresh counts the actors of at most 50 types,
8 at a time, within 10 s; the rest show "not counted". No `.app` bundle, signing or notarising is provided.

## 2. `context_client` for hosts

**Where.** `docs/guide/app.md`, `Server and credentials`, after the order of server, context and token.
**Text.** `cicada_app::context_client(&env, Some("prod"))` gives an `ActorClient` for a context of the config file,
resolved exactly as the operator commands resolve their target, with their 30 s timeout, and the name of the context
it really uses (`None` when only `CICADA_SERVER` names a server). `None` as the context picks it as `cicada` does.
**Caveats.** It checks the server URL first (`invalid server URL` or `the server URL must use http or https`), so no
error quotes it. `cicada_app::hide_passwords` hides URL passwords in a text as the usage errors do; it leaves alone a
password with a raw `/`, `#`, `?`, `"`, `` ` `` or a space.

## 3. The docs index

**Where.** `docs/README.md`, the line or table that lists the app guide's topics.
**Text.** "Inspect a running system from a Mac: the inspector window", linking `guide/app.md#inspector-on-macos`.

## 4. The crate in the crate lists

**Where.** and **Text.**
- `README.md`, the crate table after `cicada-tray`: "A native inspector window for macOS | `cicada-inspector` |
  Inspector on macOS", linking `docs/guide/app.md#inspector-on-macos`.
- `docs/architecture.md`, the crate table after `cicada-tray`: "`cicada-inspector` | A native macOS inspector over the
  HTTP API (ADR 0076): a portable model (the bounded Overview refresh, the context session, the sidebar's navigation)
  and an AppKit window behind `cfg(target_os = "macos")`; on `cicada-client` and `cicada-app`
  (`context_client`); no library crate depends on it but `cicada-tray`'s optional feature `inspector`, which the gate
  checks".
- `AGENTS.md`, the map after `crates/cicada-tray`: "`crates/cicada-inspector` | The inspector (ADR 0076): `src/model.rs`,
  the portable Overview refresh, `Session` and `Navigation`; `src/appkit` (`appkit.rs`, `appkit/content.rs`), the
  macOS window; `src/tests.rs` tests the model against an in-process server; no other library crate depends on it but
  `cicada-tray`'s feature `inspector`". The `Build and test` block may name `cargo test -p cicada-inspector`.
- `docs/how-to/install.md`, after "Run a menu bar worker": "Inspect a system from a Mac | Add `cicada-inspector` from
  the same tag and call `Inspector::new().run()` from a binary (app guide)", linking `../guide/app.md#inspector-on-macos`.
- The consumer skill `skills/cicada/SKILL.md`: add `cicada-inspector` to the description's crate list and a route row
  "An inspector window on macOS | `docs/guide/app.md#inspector-on-macos`, `crates/cicada-inspector`" once the guide
  section exists (the skill test checks that routes resolve). The builder of a later inspector change can add it.

**Caveats.** ADR 0076 is in `docs/adr/` with its row; ADR 0011's status line names it.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --test docs` (11 passed), `--doc` (54 passed) and `--test skill` (18 passed) on this branch,
with this handoff in place (its proposed links are code spans, so the link check does not follow them): nothing fails,
as no guide, README or skill route names the new crate yet. All text above is proposed; no protected prose is written.
