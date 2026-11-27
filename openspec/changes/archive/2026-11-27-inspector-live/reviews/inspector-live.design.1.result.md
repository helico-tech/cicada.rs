Verdict: not yet

Blocking findings

- Decision 1. `context_client` imposes a 30 s total deadline through body EOF. Specify a separate streaming transport with equivalent auth/TLS, a bounded handshake, 45 s since the last byte (including comments), and unchanged ordinary deadlines. Include `cicada-app` if its factory changes. Define `Stream<Item = ClientResult<Change>>`, status/MIME validation, malformed-frame policy and buffer limits. Test fragmented BOM, CRLF/CR/LF, newline-joined data and incomplete EOF. [HTML Standard](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream)
- Decisions 2–4, 6. Use server search prefixes for Actors/Messages/Outcomes/Turns, empty elsewhere. Local filters must not reconnect. Children and linked messages can fall outside the prefix. Require timer/resync pane reconciliation for those, Nodes, and mailbox/lease/watch changes that emit no hints. Coalesce initial resync with reconnect/resume. Define Live and backoff reset around the first valid event since headers/comments precede readiness. Retain bounded pending work during reads and serialize with manual paging/refresh. Run the 30 s fallback during reconnect, specify its reset point, and fence in-flight automatic work on pause while allowing manual reads. Replace the one-second pane guarantee with a scheduling bound under stated conditions.
- Decision 5. Five root pages can return full actor states, plus hundreds of child reads and pane fan-out. Bound requests, concurrency and elapsed time, including duplicate-only pages. Publish rows and cursors together. First child pages and the 500-row cap can hide surviving selections, anchors and nested folds. Distinguish unrefreshed from absent. Specify the over-budget/failure fallback and protect newer selection, scroll, folds, linked panes, and generation-bound Show Actor/Retry from background replacement.

Split into SSE transport, bounded state-preserving refresh, then live scheduling and toolbar.

Notes. Use virtual time with injected chunks and blocked reads, loopback for auth/EOF, and AppKit checks for scroll/folds. Update the architecture’s client limitation in the handoff.

No runtime or AppKit checks run. OpenSpec strict validation passed.
