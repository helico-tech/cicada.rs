# Design

## Scope and guide

1. **Principle, then Cicada form, E1** (owner, 2026-11-16). Add `docs/guide/design.md`: a small domain story explains
   the design reason, followed by its Cicada expression and links. Keep API instruction in the existing guides;
   a framework survey and Cicada-only recipes would obscure the design choices. ADR 0050 extends ADR 0041.
2. **One checked example, F2** (owner, 2026-11-16). A websocket session separates durable progress from the host's
   live socket, including reconnect to another host without actor pinning. No library API/guarantee/schema change.
   Five implementation tasks suffice: session actor/protocol, bridge/demo, tests, guide wiring/routes, ADR/handoff.
3. Guide headings: `From problem to design` (the nine steps and each written result), `Choose actors and identities`,
   `Design protocols`, `Coordinate work and time`, `Recover from failure`, `Work with outside systems`,
   `Choose a pattern`, `Recognize design mistakes`, `A websocket session`. Use actor/message/watch maps, state tables
   and lifecycle diagrams, and success/failure sequences; distinguish host calls from committed actor messages.
   Reuse saga, reply, timer, flows and supervision examples. The new section walks one websocket story end to end.
4. Explain ownership/invariants, creation and key reuse, bounded state, late replies/deadlines, business refusal vs
   failure and recovery ownership. Teach separate application DB writes as outside effects; sender checks as protocol
   validation, not authentication; typed reply refs or message.from; bounded deferred work and state/message tables.
   Link existing APIs for these and hand-written breaker/group-restart patterns rather than building more examples.
   Qualify research claims: one logical operation submitted twice can still need deduplication; logical wait cycles
   are possible; host-held dependencies can contain memory; G4 does not deduplicate arbitrary outside effects.
   Review against current architecture and ADRs 0018/0024/0034/0041/0045; research is input, not shipped authority.

## Example protocol (application convention, not a Cicada guarantee)

5. **G1: refuse when full; H1: one logical client per session** (owner, 2026-11-16). Independent tabs/devices get
   different keys. Use `examples/src/websocket_session.rs`, its tests, and `examples/examples/websocket_session.rs`.
   The host precreates a session with a unique, never-reused key for one stream life; reconnect uses that key.
   Delete ends the promise and requires a new key. No automatic expiry, restart supervisor or reset at the old key.
6. State is highest assigned sequence, cumulative ack (both initially 0), and <=100 retained `{seq, body}` frames.
   Body is <=1024 UTF-8 bytes with no NUL; sequence is 1..=9007199254740991, exact in browser JSON numbers. Enqueue
   carries body and a typed producer reply address. Acceptance appends one frame and sends `Accepted {request, seq}`
   atomically; request is the input MessageId. Reply `Rejected {request, reason}` for InvalidBody (oversized but
   otherwise storable body), BufferFull or SequenceExhausted, in that order, without advancing sequence. NUL is
   refused before enqueue/buffering as OpError::Invalid or TurnError::Invalid, with no session reply. No eviction
   or automatic retry on refusal.
   A missing reply actor rejects the commit normally. A tiny demo producer keeps at most one pending request/result.
   Transport acceptance of Enqueue is not Accepted; producer retries an explicit refusal with a new message id.
   Unknown transport outcomes use the same id within Cicada's dedup window, not a new logical enqueue. This example
   does not promise permanent business deduplication, bounded mailboxes or delivery after actor removal/purge.
7. Socket JSON uses tagged `Hello {stream, acked}`, `Frame {stream, seq, body}` and inbound `Ack {stream, through}`.
   Ack goes to the session as an ordinary message. The actor ignores wrong streams, nonpositive/out-of-range values
   and acks above the assigned sequence; <=current ack is a no-op; otherwise it drops frames <=through and advances
   ack in one commit. Client asserts contiguous processing, not just receipt; no per-socket sent cursor is trusted
   as durable evidence. The authenticated logical client is trusted to acknowledge honestly, not another subscriber.
8. Client persists its contiguous processed cursor per unique stream, ignores duplicates and acknowledges them again.
   A cursor lost behind Hello.acked requires application recovery/new session; acknowledged frames are not history.
   The host may replay from the durable ack on every refresh. Client applies only the next contiguous frame, never
   skips a gap, and retries the connection on a gap. Its local connection generation ignores old-socket callbacks.
   An old socket may still send bytes; monotone acks from the same logical client are harmless. No socket fencing,
   connection registry, shared subscribers or exactly-once client processing promise. Replays require a live store,
   running session handlers, reconnecting/cooperative client and no explicit actor/mailbox removal.

## Host bridge and verification

9. Host owns the socket and uses public ActorSystem reads/sends/changes. Subscribe before the first read, read even
   if the initial Resync has not arrived, and reread on exact-address Changed, Resync and a one-second timer. Prefix
   subscriptions are literal: filter exact addresses. Each read supplies at most 100 frames, sent in sequence above
   its ack; hints carry no frame data/history. Successful writes never trim state. Later reads may repeat frames.
   Missing session, store read/send error or closed feed closes the socket; reconnect starts from durable state.
   Socket writes and store reads/sends have a five-second bound; no unbounded channel or detached task. Read
   client acks concurrently with writing; close/error/shutdown stops and joins both sides. Invalid/binary application
   input closes; websocket controls use the library. An unknown ack-send outcome closes; replay is safe.
10. A small runnable axum adapter enables ws only for examples and uses the existing tokio/futures dependencies;
    add a matching websocket client dev dependency for the test. Bound inbound messages/frames to 4 KiB and write
    buffering to 16 KiB with eager writes. The demo binds loopback, precreates a fresh stream and a live producer,
    emits a few frames and prints connection details. Show a minimal browser client with the cursor/reconnect rules;
    host authentication/authorization of the session is an integration boundary, not provided by its key or sender.
11. Use paused-time TestActors for acceptance/refusal (including byte/count/numeric boundaries), separate NUL
    submission errors (OpError::Invalid/TurnError::Invalid, no queued Enqueue or session reply), ack monotonicity,
    duplicate/stale/wrong-stream acks, rollback when reply target is missing, and freeing capacity. Test the bridge
    through a bounded deterministic transport using the same protocol/loop as axum: disconnect before ack, lost
    ack submission, reconnect via a second ActorSystem sharing the store, overlaps, resync/missed hints and polling,
    stalled I/O, deletion, store errors/end and shutdown. One actual websocket handshake/JSON exchange checks the
    adapter; run it with TestActors and paused time, without real-time sleeps. Do not add production fault hooks.

## Documentation ownership and finishing

12. The docs session must prepare the new guide and navigation in root README.md (`Documentation`), docs/README.md
    (`Solve a task`), docs/agents.md (`Read only what the task needs`) and docs/guide/core.md (`Design an actor`,
    `Examples`); preserve old anchors.
    Builder adds DesignGuideDoctests in examples/build.rs and src/lib.rs, exports the example, and routes the existing
    skill to all nine new headings and the worked example. Extend skill REQUIRED_ROUTES; no new consumer skill yet.
    New guide include/routes block checks until the docs owner's exact files are integrated (ADR 0045). Handoff
    must list these blockers, the example/tests, submission-error versus application-refusal boundary and protocol
    caveats. Name G2's row for the new acceptance/rollback
    tests; no new G/S guarantee or generated API change. The docs owner adds the proving test names to that row.
13. Finishing checks: targeted example, guide doctests, docs/skill tests, strict OpenSpec validation, and the full
    committed-tree gate including PostgreSQL and Docker. No library implementation, broad new pattern framework,
    skill packaging or design/review-mode workflow is part of this change.

Axum adapter basis: [ws module](https://docs.rs/axum/latest/axum/extract/ws/index.html) and
[upgrade limits](https://docs.rs/axum/latest/axum/extract/ws/struct.WebSocketUpgrade.html), axum 0.8.9, checked
2026-11-16. The bridge's durable protocol is ours; websocket transport alone makes none of these replay promises.
