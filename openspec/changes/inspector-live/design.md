# Design

The inspector refreshes by hand. Operators watching a running system want the screens to follow it (owner's proposal;
the mockup's Live pill). `client-change-stream` gives `ActorClient::changes`, a stream of change hints that never
reconnects by itself; `inspector-soft-refresh` and `inspector-soft-refresh-actors` give every page a bounded reload that
keeps the view. This change schedules them: one stream for the page on screen, reloads from its hints and a slow timer,
and the Live pill.

## Decisions

1. **One stream for the page on screen.** The inspector keeps one change stream at a time, opened with
   `ActorClient::changes` from the session's client: under the page's search prefix on Actors, Messages, Outcomes and
   Running Turns (the prefix the server lists with; a local filter, as on Types and Watches, leaves the subscription
   alone), and under the empty prefix on the Overview, Types, Watches and Nodes. A page switch or a new prefix (after
   the search's 300 ms) closes it and opens the next; a context switch or a closed window closes it.
2. **The server's resync is the one recovery trigger.** Every new subscription starts with the server's `resync` (ADR
   0035); that event, not the opening, triggers the soft refresh after an open, a reconnect or a resume, so a stream
   that never gets going causes no extra reload. A stream that ends (`None`, a store close) or fails (any
   `ChangeStreamError`, or an error opening it) is opened again after 1, 2, 4, 8, 16, then 30 s between tries; the
   backoff resets once a stream has stayed open for 60 s after its first valid event.
3. **The live model owns the batch.** Hints are gathered for 500 ms after the first of a batch, into a set of addresses
   bounded at 1,000 (past that the batch counts as "everything", as a `resync` does). The live model starts the page's
   soft refresh only when none is running, and gathers what arrives meanwhile into the next batch, which starts once the
   refresh ends (at most about two a second); it never relies on the soft refresh's own remembered request. On the paged
   pages with an actor per row, a batch that names no address under the page's prefix and no actor of the pane starts no
   refresh; on the other pages any batch does.
4. **Every soft refresh reconciles the pane.** A soft refresh, from a batch, the timer or a resync, brings the pane in
   line by the soft refresh's own rules (a pane from its row is refilled, a reading pane is read again without
   "Loading"; `inspector-soft-refresh` re-reads a selected node's pane, `node(id)`, the same way). Hints the stream
   cannot carry, such as an Actors child outside the search prefix or an Outcomes link to a message elsewhere, are
   caught by the timer.
5. **A slow timer.** While live, the page runs a soft refresh 30 s after its last refresh of any kind ended (soft,
   timer, resync or manual); the timer is armed again whenever a refresh attempt ends, also one the soft refresh skipped
   (while paging has failed) or that missed its deadline, and keeps running while the stream reconnects or waits for its
   first event, so a page stays fresh without hints.
6. **The Live pill and pause.** The toolbar shows Live (from the stream's first valid event), Reconnecting (between
   tries; the tooltip says when the next one is and why the last ended) or Paused. Clicking it pauses automatic
   refreshes: the stream closes, the timer stops, the gathered batch is cleared, and a soft refresh in flight is
   cancelled, with its list and pane results fenced so none applies. Clicking again resumes: a stream opens, and its
   first `resync` runs one soft refresh. A manual Refresh (⌘R) works in every state and re-arms the timer.
7. **Where it lives.** The live model in `live.rs` (the stream's lifecycle, backoff, the batch, the timer, pause) on the
   pages' `soft_refresh`, with tests on virtual time against a fake change source (a private async trait the model uses,
   implemented over `ActorClient::changes` in the app and by a channel in tests); the pill in the `appkit` module.

## Tests

On virtual time with a fake change source: one stream per page and prefix, closed on page, prefix and context switches
and on close, and a local filter leaving it alone; the first `resync` of an open, a reconnect and a resume each causing
one soft refresh, and a stream with no event causing none; the backoff and its reset 60 s after the first valid event;
fifty hints in 200 ms causing one refresh; hints during a refresh causing exactly one more; a batch over 1,000 addresses
counting as everything; a batch outside the prefix starting none on a paged page; the pane reconciled by every soft
refresh; the timer armed after every refresh attempt, a skipped one included (recovery after a Retry with no hints), and
running while reconnecting, catching an Actors child outside the prefix and an Outcomes link elsewhere; pause closing
the stream, clearing the batch and fencing a refresh in flight, resume's `resync` causing one refresh; ⌘R re-arming the
timer. Against an in-process server: a real change starting the Actors page's soft refresh within a second of the hint's
receipt on an idle page; Live shown from the first valid event. The pill gets a manual check list in the tasks.

## Docs

The handoff gives the app guide's Inspector section the Live pill, what Reconnecting and Paused mean, and that updates
are best effort with a 30 s fallback; the changelog's 0.26.0 section names live updates.
