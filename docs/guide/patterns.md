# Actor examples, visually

Follow the messages to see where work runs and where progress becomes durable.
Arrows between actors are queued messages; external calls happen inside a handler and can repeat after a crash.

## Paged import

[sync.rs](../../examples/src/sync.rs) processes one page per turn. Each commit saves the count and queues the next page;
the last page reports the total and schedules the next run.

```mermaid
flowchart TD
    start["Run"] --> reset["Reset imported count"]
    reset --> page["FetchPage turn"]
    page --> api["Fetch one external page"]
    api --> commit["Commit count + next message"]
    commit -->|more pages| page
    commit -->|finished| reports["Reports actor"]
    commit -->|finished: delayed Run| start
```

The external fetch uses the message id as its idempotency key. The diagram's cycle is a chain of committed turns,
not an in-memory loop that must stay alive.

## Fan-out and fan-in

[fanout.rs](../../examples/src/fanout.rs) creates one child per item in the parent's turn.
The children can run in parallel; the parent handles their replies one at a time.

```mermaid
sequenceDiagram
    participant P as Batch parent
    participant S as Shared SQL store
    participant A as Item 0
    participant B as Item 1
    P->>S: Commit children + ProcessItem sends
    S->>A: Deliver ProcessItem
    S->>B: Deliver ProcessItem
    par Independent child turns
        A->>S: Commit ItemDone + stop child
    and
        B->>S: Commit ItemDone + stop child
    end
    S->>P: Deliver ItemDone messages serially
    Note over P: Save results<br/>by child index
```

Reply order is unspecified. The parent counts each index once; deleting it removes any children still running.

## Request and reply

[reply](../../examples/src/reply/) uses two ordinary messages. The caller records progress in state instead of
keeping a handler open while waiting.

```mermaid
sequenceDiagram
    participant C as Caller actor
    participant R as Responder actor
    C->>R: Request message
    Note over C: Commit and finish this turn
    R->>C: Reply to message.from, same correlation id
    Note over C: Handle reply in a new turn
```

This is a message convention, not a synchronous ask API. The responder knows the return address from the request.

## Durable flows

[flow_onboarding.rs](../../examples/src/flow_onboarding.rs) saves one new step result or sleep per turn.
Flows can also [send actor work](actors.md#sending-to-actors) atomically with a new journal entry.
Every execution starts at the top; saved entries replay without repeating their closures.

```mermaid
flowchart TD
    replay["Replay saved entries"] --> next{"Next entry?"}
    next -->|new step| executeStep["Run closure with stable key"]
    executeStep --> commit["Commit result + Continue"]
    commit --> replay
    next -->|new sleep| sleep["Commit deadline + delayed Continue"]
    sleep -->|deadline reached| replay
    next -->|new send| send["Commit optional create + message<br/>+ entry + Continue"]
    send --> replay
    next -->|none| done["Keep completed run + results"]
```

The closure can repeat if its turn never commits; the external service must deduplicate by the step key.
The send branch enqueues once per committed entry; it does not wait for the target to succeed.
See [flow rules](actors.md#flows) before changing entry ids, order or result types.

For ready-made composition, see [supervision](actors.md#supervisor), [message processing](actors.md#message-processing),
and [events starting flows](actors.md#event).
