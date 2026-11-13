# Design

## Scope

One fixed actor `EventActor` under `cicada.event`, with `Event { name }` and `Event::named`. The owner's Opus 5.5 adviser
settled these policies, including direct batcher composition. Four implementation tasks plus finishing checks.
It uses FlowState and Continue from durable-flows/ADR 0031 on the public core API; no core or existing contract changes.

## Decisions

### An explicit subscription list

`EventState { flows: Vec<String> }` defaults to empty. State decoding validates at most `MAX_FLOWS = 100` unique names,
each accepted by TypeName::parse; preserve subscription order. Fields are public; unknown fields are ignored.
`EventMsg` is tagged/camelCase: `Subscribe { flow: String }`, `Unsubscribe { flow: String }`,
`Publish { input: Value }`, `PublishBatch { items: Vec<Value> }`. Subscribe/Unsubscribe parse the name and refuse invalid
ones permanently. Subscribe is a no-op for an existing name even at capacity; a new 101st name is refused. Unsubscribe
is a no-op when absent and never deletes existing runs. Subscribe defers registration/role/input checks until Publish.
Hosts create the event actor and explicitly subscribe at startup, or create-and-subscribe in one turn; no App hook.
Not chosen: automatic subscription (hidden startup effects) or reusing Topic (it sends to existing instances).

### Two unambiguous publication shapes

Publish passes its input unchanged, including scalar, array or null. Missing input fails decoding. PublishBatch is
exactly Publish with input `{"items": items}`; it starts one run per flow for the entire array, including an empty one.
Batcher sets messageType to PublishBatch, so its existing `{"items": [...]}` payload fits directly. Nonarray items
fail decoding. `EventMsg::publish(input: impl Serialize) -> Result<Self, EncodeError>` serializes input;
`publish_batch<T: Serialize>(items: impl IntoIterator<Item=T>) -> Result<Self, EncodeError>` serializes each item.
Helpers do no extra depth validation. No shared Batch<T> type: hosts declare their own Input with an items field.
Not chosen: flattening Publish input (loses general JSON input), serde aliases (unclear schemas), or a host adapter.

### Atomic create and start

For each subscribed type, take the current publication envelope id as the raw key and construct its canonical address.
Use ActorAddress::of, never parse an interpolated address. Address-construction failure is permanent (refused):
literal `.` and `..` ids cannot name runs. With no subscribers no address is needed and those ids succeed as no-ops.
Recover a refused publication using a fresh valid id (operator retry/redrive normally generates one). No prefix or
special remapping is added: raw ids remain the run identity. `a/b` maps to `a%2Fb`; raw `a%2Fb` maps to `a%252Fb`.
Buffer `create_raw(address, serialize(FlowState::<Value>::new(input.clone())))`, then
`send_raw(address, "Continue", {}, options)` with the shared crate retry policy and the publication's correlation id.
Creates are independent, not children. One commit creates/starts every run or none; an empty list is done with no effects.
The initial state is `{"input":X,"journal":[],"status":"running","armed":null}`. Preserve subscription order in
buffering, but promise no execution ordering across runs. Every Continue has a new message id, the Event actor as
sender, default retention and ordinary observer headers; incoming arbitrary headers are not copied.
The event actor cannot read F::retry(); the run's normalization turn selects it before executing flow code.
Not chosen: one transaction per subscriber (partial success and recovery machinery), or a policy inferred from schemas.

### Failure boundaries and recovery

A create/send refused at the call is permanent (refused); a store error propagates so the publication envelope's
policy controls retries. Any commit rejection follows core semantics. No partial creates, starts or event-state changes
commit on failure. Stale unregistered subscriptions block publications; unsubscribe them before retrying/redriving.
The event actor watches nothing; hosts watch events for failed publications and flow types for failed run messages.
Once Publish commits, later failures of Continue or flow code are run failures, never an event rollback.

Only registry acceptance of the type and Continue can be checked on every node. A hosting node additionally decodes
the prospective state/input at create_raw; refusal aborts the whole publication. A non-hosting node cannot decode it:
invalid input can commit and then fail with cause state at the run, while other runs proceed. The registry cannot prove
that an actor is FlowActor: subscribe only flow types. An ordinary actor lacking Continue is refused; one accepting it
may receive this state and message. Do not promise semantic input validation or successful flow execution in the fan-out.

### Identity and deduplication

Core policy retries keep the publication id and hence run addresses. A duplicate incoming id within the core window
adds nothing. After that window, resending the same id can run Publish again: existing run creates are no-ops and keep
first input/progress; done ignores Continue, running normalizes recovery. Calls still validate the prospective state
before a repeated create, so an invalid replacement input can refuse even when a run already exists. New subscribers
may get new runs and deleted addresses may be recreated. No permanent event ledger is kept.
Operator retry and dead-letter redrive use NEW message ids, hence new run addresses. A failed event turn committed no
starts to duplicate; repeated operator retry calls can deliberately create multiple publications. A failed run is
recovered by its Continue, not by republishing the original event. An empty successful event is not replayed to later
subscribers automatically; publish again explicitly. Removing an event actor does not delete its independent runs.

Never reuse publication/run keys for different jobs, even across event names or after retention/deletion. Message ids
are unique only across pending/retained records, not forever. Manual runs of subscribed types must avoid publication
keys. Fresh scheduler slots, limiter/debouncer forwards and batcher batches get fresh ids and thus fresh runs. External
idempotency keys still follow ADR 0031, including after reusing a deleted run address for the same business job.

### Depth and integration

Publish input sits one level down in both payload and run state, so up to 63 levels fit. PublishBatch items sit two
levels down in its payload but three in run state, so each item can be at most 61 deep when creating runs. An item
62 deep can leave the batcher and reach the Event actor, then create_raw's core state-depth guard refuses the whole
publication permanently. The batcher has already cleared it; the event's failed message/dead-letter watch is recovery.
No new validator is added. With no subscribers there is no run create and no extra state-depth check.

Pin the Event contract and empty/populated state plus all message samples in wire_formats and docs/reference/contracts.
Host EventActor in the sample service with no automatic instance/subscription; keep existing flow contracts unchanged.
Add tested examples and actors-guide doctests for typed input and batch input, explicit subscriptions, watches and
recovery, scheduler/limiter/debouncer Publish and batcher PublishBatch. Update tables, README, crate docs, architecture,
AGENTS map and changelog. No new ADR: a fixed actor following ADR 0028/0031 adds no change to existing guarantees or wire.
Tests use virtual-time TestActors, including hosting/non-hosting nodes sharing a public store; reuse the SQLite
registry-failure fixture only when needed. Check against the landed runner and batcher before implementation.

Review 1 found the missing address-error policy for dot ids. The refusal, empty-list and fresh-id recovery scenarios
now cover it, plus slash/percent encoding. This completes the existing raw-id choice without changing the contract.
