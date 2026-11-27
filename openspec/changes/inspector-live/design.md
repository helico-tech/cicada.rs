# Design

The inspector refreshes by hand. Operators watching a running system want the screens to follow it (owner's proposal;
the mockup's Live pill). The server already streams change hints: `GET /changes?prefix=` (ADR 0035) sends `changed`
with an actor's address and `resync`, best effort, with keep-alive comments every 15 s and no replay. `cicada-client`
has no reader for it yet.

## Decisions

1. **A change stream in `cicada-client`.** `ActorClient::changes(prefix) -> ClientResult<ChangeStream>`, an async
   `Stream` of `Change::Changed(ActorAddress)` and `Change::Resync`, parsed from the event stream (event and data lines;
   comments ignored; an unknown event skipped). It sends the client's headers and token, and has no total timeout:
   instead the stream ends with an error when nothing, not even a keep-alive, arrives for 45 s (three keep-alive
   periods). It never reconnects by itself; the caller decides. A store close ends the stream without an event, as ADR
   0035 says. Additive, so no break; `docs/openapi.json` does not change.
2. **One stream for the page on screen.** The inspector keeps one stream at a time, for the visible page's prefix:
   the search prefix on Actors, Messages, Outcomes and Running Turns, and the empty prefix (every actor) on the
   Overview, Types, Watches and Nodes. Switching pages or changing the prefix closes the stream and opens the next; a
   context switch or a closed window closes it. A stream that ends or fails is opened again after 1, 2, 4, 8, 16, then
   30 s between tries.
3. **Hints, coalesced.** Hints are collected for 500 ms after the first of a batch; then one reload of the page runs,
   and no second one starts until it ends (at most about two a second). A `resync`, a reconnect and the first event of a
   new stream each also reload the page. The pane is read again when a batch names its actor (the selected actor, the
   actor of the selected message, outcome or turn, a watch's watcher or target actor) or holds a `resync`. Pages without
   an actor per row (Types, Nodes, the Overview) reload on any batch.
4. **A slow timer.** While live, the page also reloads every 30 s after its last reload, as hints are best effort.
5. **Reloads keep what the user sees.** A live reload is a soft refresh: it re-reads the loaded range (the root pages
   loaded so far, at most the first 500 rows; the open children's first pages) under a new generation, and then puts
   back the selection (by id or path), the scroll position (the first visible row, by identity, or the same offset
   when that row is gone) and the open rows (by `inspector-children`' rule). A selection whose row is gone is cleared,
   as a manual Refresh does. Show Actor's waiting jumps and Retry rows are left as they are.
6. **The Live pill.** The toolbar shows Live (stream open), Reconnecting (between tries, with the next try's time in
   its tooltip) or Paused. Clicking it pauses (the stream is closed, the timer stops, nothing reloads by itself) and
   resumes (a stream is opened and the page reloads). Pause lasts until resumed or the window closes; manual Refresh
   (⌘R) works in every state.
7. **Where it lives.** The stream in `cicada-client` with tests against an in-process server (`cicada-axum`'s
   router); the live model in the inspector's `live.rs` (stream lifecycle, backoff, coalescing, the timer, the soft
   refresh's restore) with tests on virtual time; the pill in the `appkit` module.

## Tests

Client: `changed` and `resync` parsed, comments and unknown events skipped, the 45 s idle end, a store close ending the
stream, headers and token sent. Inspector: one stream per visible page and prefix, closed on a page or prefix change, a
context switch and close; reconnect backoff; hints coalesced into one reload, at most one in flight; the pane read again
when its actor is named; a reload on `resync`, reconnect and a new stream; the 30 s timer; pause and resume; a soft
refresh keeping the selection, the scroll position and open rows, and clearing a selection whose row is gone. Tests run
on virtual time with a wall-clock bound.

## Docs

The handoff gives the HTTP guide `ActorClient::changes` and the app guide's Inspector section the Live pill; the
changelog's 0.26.0 section names both.
