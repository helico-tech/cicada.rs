# Design

The inspector refreshes by hand. Operators watching a running system want the screens to follow it (owner's proposal;
the mockup's Live pill). `client-change-stream` gives `ActorClient::changes`, a stream of change hints that never
reconnects by itself; `inspector-soft-refresh` and `inspector-soft-refresh-actors` give every page a bounded reload that
keeps the view. This change schedules them: one stream for the page on screen, reloads from its hints and a slow timer,
and the Live pill.

## Decisions

1. **One stream for the page on screen.** The inspector keeps one change stream at a time, opened with
   `ActorClient::changes` from the session's client: under the page's search prefix on Actors, Messages, Outcomes and
   Running Turns (the prefix the server lists with), and under the empty prefix on the Overview, Types, Watches and
   Nodes. A page switch or a new prefix (after the search's 300 ms) closes it and opens the next; a context switch or a
   closed window closes it.
2. **Reconnects.** A stream that ends (`None`, a store close) or fails (any `ChangeStreamError`, or an error opening it)
   is opened again after 1, 2, 4, 8, 16, then 30 s between tries; a stream that stayed open for 60 s resets the backoff.
   The first event of a new stream, and every reconnect, count as a `resync`.
3. **Hints into soft refreshes.** Hints are gathered for 500 ms after the first of a batch, then the page's soft refresh
   runs; while it runs, new hints gather into the next batch, which runs once when it ends (at most about two soft
   refreshes a second). A `resync` in a batch is the same as a hint for everything. On the paged pages with an actor per
   row, a batch that names no address under the page's prefix and no actor of the pane starts no refresh; on the other
   pages any batch does. A batch that names the pane's actor (the selected actor; the actor of the selected message,
   outcome or turn; a watch's watcher or target) also re-reads the pane, as the soft refresh does.
4. **A slow timer.** While live, the page also runs a soft refresh 30 s after its last refresh of any kind, as hints are
   best effort; the timer keeps running while the stream reconnects, so a page stays fresh without hints.
5. **The Live pill.** The toolbar shows Live (stream open), Reconnecting (between tries; the tooltip says when the next
   one is and why the last ended) or Paused. Clicking it pauses: the stream closes, the timer stops, gathered hints are
   dropped, and a soft refresh in flight is cancelled (its results do not apply). Clicking again resumes: a stream opens
   and the page runs one soft refresh. Pause lasts until resumed or the window closes; a manual Refresh (⌘R) works in
   every state and resets the timer.
6. **Where it lives.** The live model in `live.rs` (the stream's lifecycle, backoff, batching, the timer, pause) on the
   pages' `soft_refresh`, with tests on virtual time against a fake change source (a trait the model uses, implemented
   by `ActorClient::changes` in the app and by a channel in tests); the pill in the `appkit` module.

## Tests

On virtual time with a fake change source: one stream per page and prefix, closed on page, prefix and context switches
and on close; the backoff and its reset after 60 s open; a new stream and a reconnect each causing one soft refresh;
fifty hints in 200 ms causing one refresh; hints during a refresh causing exactly one more; a batch outside the prefix
starting none on a paged page; the pane re-read for its actor; the 30 s timer, also during reconnects; pause closing the
stream, dropping hints and discarding a refresh in flight, resume causing one refresh; ⌘R resetting the timer. Against
an in-process server: a real change reaching the Actors page's pane within a second. The pill gets a manual check list
in the tasks.

## Docs

The handoff gives the app guide's Inspector section the Live pill, what Reconnecting and Paused mean, and that updates
are best effort with a 30 s fallback; the changelog's 0.26.0 section names live updates.
