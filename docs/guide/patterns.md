# Actor examples, visually

Follow the messages to see where work runs and where progress becomes durable.
Arrows between actors name queued message variants. Dashed arrows to/from an external API are calls and return values,
not actor messages; those calls can repeat after a crash.

## Paged import

[sync.rs](../../examples/src/sync.rs) processes one page per turn. Each commit saves the count and queues the next page;
the last page reports the total and schedules the next run.

```mermaid
sequenceDiagram
    participant U as Caller
    participant S as Sync
    participant A as External API
    participant R as Reports
    U->>S: SyncMsg::Run
    Note over S: Commit reset + FetchPage
    S->>S: SyncMsg::FetchPage { cursor: None }
    loop One page per turn
        S-->>A: SyncApi::fetch (external call)
        A-->>S: Page (return value)
        alt More pages
            Note over S: Commit count + FetchPage
            S->>S: SyncMsg::FetchPage { cursor: Some(next) }
        else Last page
            Note over S: Commit count + SyncFinished<br/>+ delayed Run
            S->>R: ReportsMsg::SyncFinished { imported }
            S->>S: SyncMsg::Run (after 1 hour)
        end
    end
```

The external fetch uses the message id as its idempotency key. The diagram's cycle is a chain of committed turns,
not an in-memory loop that must stay alive.

## Fan-out and fan-in

[fanout.rs](../../examples/src/fanout.rs) creates one child per item in the parent's turn.
The children can run in parallel; the parent handles their replies one at a time.

```mermaid
sequenceDiagram
    participant U as Caller
    participant P as Batch parent
    participant A as Item 0
    participant B as Item 1
    U->>P: BatchMsg::StartBatch { items }
    Note over P: Create children + sends<br/>in one commit
    P->>A: BatchItemMsg::ProcessItem
    P->>B: BatchItemMsg::ProcessItem
    par Independent child turns
        A->>P: BatchMsg::ItemDone { index: 0, result }
        Note over A: ItemDone + stop<br/>commit together
    and
        B->>P: BatchMsg::ItemDone { index: 1, result }
        Note over B: ItemDone + stop<br/>commit together
    end
    Note over P: Handle ItemDone serially<br/>save each index once
```

Reply order is unspecified. The parent counts each index once; deleting it removes any children still running.

## Request and reply

[reply](../../examples/src/reply/) uses two ordinary messages. The caller records progress in state instead of
keeping a handler open while waiting.

```mermaid
sequenceDiagram
    participant U as Caller
    participant C as Cart
    participant P as Pricing
    U->>C: CartMsg::AddItem { sku, quantity }
    Note over C: Commit awaiting entry<br/>+ QuoteRequest
    C->>P: PricingMsg::QuoteRequest { sku, quantity }
    alt Price available
        P->>C: CartMsg::Quote
    else Invalid quantity or unknown SKU
        P->>C: CartMsg::QuoteRejected
    end
    Note over C,P: Reply goes to message.from<br/>with the same correlation id
    Note over C: Reply handled<br/>in a new turn
```

This is a message convention, not a synchronous ask API. The responder knows the return address from the request.

## Durable flows

[flow_onboarding.rs](../../examples/src/flow_onboarding.rs) saves one new step result or sleep per turn.
Flows can also [send actor work](actors.md#sending-to-actors) atomically with a new journal entry.
Every execution starts at the top; saved entries replay without repeating their closures.

```mermaid
flowchart TD
    start["Caller"] -->|FlowMsg::Continue| replay["Replay saved entries"]
    replay --> next{"Next entry?"}
    next -->|new step| executeStep["Run closure with stable key"]
    executeStep --> commit["Commit result + Continue"]
    commit -->|FlowMsg::Continue| replay
    next -->|new sleep| sleep["Commit deadline + delayed Continue"]
    sleep -->|"FlowMsg::Continue (when due)"| replay
    next -->|new send| send["Commit optional create + send<br/>+ entry + Continue"]
    send -->|ReportsMsg::SyncFinished| reports["Reports actor"]
    send -->|FlowMsg::Continue| replay
    next -->|none| done["Keep completed run + results"]
```

The closure can repeat if its turn never commits; the external service must deduplicate by the step key.
The send branch uses `ReportsMsg::SyncFinished` from [the send example](actors.md#sending-to-actors). It enqueues once
per committed entry and does not wait for the target to succeed.
See [flow rules](actors.md#flows) before changing entry ids, order or result types.

## Flow requests and replies

[flow_approval.rs](../../examples/src/flow_approval.rs) asks an actor for a human decision. Unlike the ordinary
cart/pricing protocol, the flow library saves the request and wait outcome. This purchase sets a one-week decision deadline.

```mermaid
sequenceDiagram
    participant F as Purchase flow
    participant A as Approvals
    participant U as Human / caller
    Note over F: Commit Ask + entry<br/>+ Continue
    F->>A: ApprovalsMsg::Ask (ReplyTo route)
    F->>F: FlowMsg::Continue
    Note over F: Save open wait<br/>+ deadline Continue
    alt Reply handled before timeout commits
        U->>A: ApprovalsMsg::Decide
        A->>F: FlowMsg::Reply { request, value }
        Note over F: Save reply + close wait<br/>Cancel deadline timer<br/>+ arm Continue
        F->>F: FlowMsg::Continue
        Note over F: Return Waited::Received<br/>with Decision
    else Deadline handled first with no reply
        F->>F: FlowMsg::Continue (deadline)
        Note over F: Commit TimedOut<br/>+ arm Continue
        F->>F: FlowMsg::Continue
        Note over F: Return Waited::TimedOut
    end
```

Only the Continue after an outcome commits returns it to flow code. The target must send the reply itself.
Mailbox order decides the race: a reply handled first can win after the deadline. Timeout leaves target work running.
See [request and wait rules](actors.md#requests-and-reply-waits) for ignored replies and recovery.

For ready-made composition, see [supervision](actors.md#supervisor), [message processing](actors.md#message-processing),
and [events starting flows](actors.md#event).
