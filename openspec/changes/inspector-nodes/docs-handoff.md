# Docs handoff for inspector-nodes

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The macOS inspector's Running Turns and Nodes items have read-only pages; no sidebar item says "Coming soon" any more.
No public API changed. The pages live in `crates/cicada-inspector/src/turns.rs` and `src/nodes.rs` (models) and
`src/appkit/turns.rs` and `src/appkit/nodes.rs` (window).

The in-process server tests in `crates/cicada-inspector/src/tests/turns.rs` cover paging 150 claimed turns by the
server's cursor while leases begin between pages (each actor once), the cursor passed back unchanged while a lease
ends between pages and an empty page followed, the pane from the row with no read, Refresh and a search clearing it, a
failed page with Retry, Show Actor, and back to a turn that ended ("no longer running"), that moved to another message,
whose read failed (Retry), and whose prefix the page no longer showed. `src/tests/nodes.rs` covers the cache read on
the first visit, on its Refresh and after a context switch (at once when visible, else at the next visit) and never on
the Overview's refresh, the status and last heartbeat as the server gave them, a late list answer after a context switch
dropped, a failed read with Retry and no stale list, the read deadline, the local id filter hiding the selection, a
node's two turns with Show Actor and back reading it fresh, a gone node, a failed node read with Retry, and node reads
dropped on A, then B, then A, on Refresh and on a context switch.

## 1. The sidebar

**Where.** `docs/guide/app.md`, `Inspector on macOS`, the paragraph that starts "The sidebar lists Overview and the
Browse items".

**Text.** Every item has a page now: drop "and the others say "Coming soon"". Running Turns and Nodes also share the
toolbar's search field and pane toggle, each keeping its own filter text; the field's placeholder says what to enter
("Address prefix" on Running Turns, "Node id prefix" on Nodes).

## 2. The Running Turns page

**Where.** `docs/guide/app.md`, `Inspector on macOS`, after Messages.

**Text.** Running Turns lists the turns running now on the actors under the address prefix in the search field, 100 at
a time as you scroll: the actor, the message id, the node that holds the lease, the lease end, and "asked" when an
operator asked to cancel the turn. The server pages turns by actor address, so an actor never shows twice, even while
turns start and end as you scroll. Select a turn to see it in the pane, read only, from its row; **Show Actor** goes to
its actor on the Actors page, where the actor's pane has its state, mailbox and watches.

Back from the actor returns to the turn and reads the actor's turn again by its address. When the actor still runs a
turn, the pane shows it as it is now, which may be another message, and its row is selected. When the turn has ended,
the pane keeps the turn as it was read and marks it "no longer running". When that read fails, the pane shows the
error with **Retry**.

**Caveats.** The list is a snapshot of when each page was read: a turn may have ended by the time you see it. ⌘R reads
the list again from the top and clears the selection. A failed page shows **Retry** in the footer. Cancelling a turn is
not in the inspector yet.

## 3. The Nodes page

**Where.** `docs/guide/app.md`, `Inspector on macOS`, after Outcomes.

**Text.** Nodes lists every node: its id, status (alive, suspect or dead), the types it hosts ("sends only" for none),
its label, host name, last heartbeat and the number of its running turns. The status is computed by the server, in the
store's clock: alive when the node was seen under 30 s ago, suspect under 2 minutes, dead after that. The last
heartbeat shows as a time, not an age, as the Mac's clock may differ from the store's. The footer says the status is as
of the last read; its tooltip gives the time of that read on the Mac's clock. The search field filters the nodes read
by id prefix, without a new read.

The page reads the nodes on its first visit, on ⌘R and after a context switch (at once when the page is shown, else on
the next visit). The Overview's refresh does not change it. A failed read shows its error with **Retry**, and no older
list.

Select a node to read it with its running turns: actor, message id, lease end and a cancel request, each with **Show
Actor**. A node whose row is gone says "not found", and a failed read shows **Retry**. Back to a Nodes place selects
the node again and reads it fresh.

**Caveats.** The status is not live: press ⌘R to read it again. ⌘R also clears the selection. Selecting another node,
or typing a filter that hides the selected one, drops a read still out. Every read gives up after 10 s.

## Blocking

None.

## Status

Ran `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`cargo test -p cicada-examples --test docs` and `--doc`, and `openspec validate inspector-nodes --strict`; results in
the build report. All text above is proposed; no prose is written yet.
