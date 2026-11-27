# Docs handoff for inspector-live

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The inspector updates the page on screen by itself. It follows one change stream (`GET /changes`, ADR 0035) for that
page, runs the page's soft refresh from the hints, and also refreshes every 30 s, as hints are best effort. The toolbar
has a Live pill that shows the state and pauses or resumes the updates. No public API changed: the live model is private
to `cicada-inspector` (`src/live.rs`). The crate's rustdoc describes it. Tests:
`crates/cicada-inspector/src/live/tests.rs` (the model on virtual time against a fake change source),
`src/tests/soft_refresh/live.rs` (the model on the session's pages against a scripted server) and `src/tests/live.rs` (a
real change from the actor API starting the Actors page's soft refresh within a second of its hint).

## 1. Live updates in the inspector

**Where.** `docs/guide/app.md`, `Inspector on macOS`, together with the soft refresh text of `inspector-soft-refresh`
and `inspector-soft-refresh-actors`.

**Text.** The page on screen follows the server as it changes. The inspector listens to the server's change hints for
that page: for the actors under the search prefix on Actors, Messages, Running Turns and Outcomes, and for every actor
on the Overview, Nodes, Types and Watches. When hints arrive, it waits half a second for more, then refreshes the page
in place: the selection, the scroll position and the open rows stay as they are, and the pane is read again without
"Loading". It refreshes at most about twice a second. On Actors, Messages, Running Turns and Outcomes, only a hint for
an actor under the prefix, or for the actor in the pane, starts one.

Hints are best effort: the server can miss one, and a child row outside the search prefix, or an Outcomes link to a
message of another actor, gets none. So the page also refreshes 30 s after its last refresh ended, with or without
hints, also while the inspector reconnects.

The toolbar's pill shows the state of the updates:

- **Live**: the server's change stream is open and has answered.
- **Connecting**: the stream is opening and has not answered yet.
- **Reconnecting**: the stream ended or failed. Its tooltip says why, and when the next try is. Tries come after 1, 2,
  4, 8 and 16 s, then every 30 s. The page still refreshes every 30 s meanwhile. A new stream refreshes the page once
  as it starts.
- **Paused**: you clicked the pill. Nothing reloads by itself, and a refresh that was running is cancelled. Click again
  to resume: the page refreshes once as the stream starts.

Refresh (⌘R) reloads the page in every state, also while paused.

**Caveats.** Each refresh has the limits of a soft refresh: four requests at once, ten seconds, at most 300 root rows
and the first children page of up to 20 open parents on Actors. An Actors refresh that a pause, leaving the page or a
Retry cancels marks the rows and the open parents it did not reach "not refreshed". Rows beyond what a refresh reads can
be older than the page's other rows. The updates need a server with `GET /changes` (0.26.0 or later): an older server
answers 404, and the pill stays on Reconnecting while the page still refreshes every 30 s.

## Blocking

None.

## Status

The wording for the app guide is proposed; no protected file was edited. The changelog's 0.26.0 section names live
updates. Checks: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed; `cargo
test --workspace` with PostgreSQL passed 4,028 tests in 138 suites, none failed; `cicada-examples` `--test docs` passed
11 and `--doc` 55; `openspec validate inspector-live --strict` passed. The manual AppKit checks of the pill are
outstanding: a node following a change on the Actors page within about a second with the selection, scroll and open rows
kept; Live, then Reconnecting when the server stops and Live again when it comes back; Paused stopping updates while ⌘R
still reloads; one reload on resume; light and dark; VoiceOver reading the pill.
