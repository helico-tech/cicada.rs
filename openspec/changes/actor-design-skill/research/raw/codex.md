# Designing systems with actors

Research for an **actor-design** skill. Research date: 15 November 2026.

Scope: modelling, responsibilities, state, protocols, coordination, failure contracts, and external interaction. No repository content was read or changed. Runtime implementation is outside scope.

**Evidence labels:** **Established** means supported by foundational work, established messaging literature, or recurring practice across systems. **Framework-specific** means a particular platform's advice or contract. **Synthesis** means guidance proposed here from the cited evidence. Pattern trade-offs, modelling questions, examples, and review checks are synthesis unless attributed otherwise. These labels prevent heuristics from being mistaken for universal actor laws. Sources were read selectively for design-relevant material. Access limitations are recorded in section 9.

An actor here is an independently addressed participant that owns decisions and state and interacts through a protocol. A durable workflow is a related tool for a conversation over time. The two abstractions overlap but are not interchangeable.

## 1. Design principles

1. **P01 — Find responsibilities, not classes or tables.** Look for an independent activity, an enduring business identity, or a resource needing one authority. Armstrong starts from real-world concurrency; Orleans starts from independently addressable entities. **Established.** [Armstrong][s03]; [Orleans practices][s14].
2. **P02 — Give every immediate invariant one decision owner.** A rule that must hold at every accepted transition needs a boundary that can enforce it. DDD aggregates help find that boundary. Mapping an aggregate to an actor is a choice, not a DDD requirement. **Established basis; mapping is synthesis.** [Vernon I][s08].
3. **P03 — Messages are the public contract.** Define permitted requests, outcomes, and conversation rules before handlers. The original actor paper separates an actor's obligations from how it fulfils them. **Established.** [Hewitt, Bishop, Steiger][s01]; [Pekko actors][s19].
4. **P04 — Keep authoritative mutable state private.** Others request decisions or receive immutable information. They do not mutate the owner's state through another path. **Established.** [Armstrong][s03]; [Scala mixing study][s48].
5. **P05 — Represent uncertainty and outstanding obligations.** What an actor remembers about a partner is local knowledge, not the partner's current state. Pending and unknown are meaningful states. **Established.** [Helland, entities and activities][s10].
6. **P06 — Separate business refusals from faults.** Invalid or declined work is an ordinary protocol outcome. A fault may invalidate the executing component and require recovery elsewhere. **Established.** [Akka introduction][s15]; [OTP supervision][s04].
7. **P07 — Design recovery responsibility, not just restarts.** Identify who restores a service and what stable condition that service promises after recovery. **Established, with framework-specific mechanisms.** [Hebert, Guarantees][s06]; [Proto.Actor supervision][s25].
8. **P08 — Sending and timing out do not establish business outcome.** A send is not proof of processing. A timeout is lack of timely knowledge, not proof of cancellation. **Established.** [Akka interactions][s16]; [Temporal activities][s28].
9. **P09 — Identify logical operations across attempts.** A retry of one intention differs from a new intention. Deduplication must cover the change it claims to protect. **Established.** [EIP Idempotent Receiver][s40]; [Stripe idempotency][s33].
10. **P10 — Local atomicity does not extend automatically to external effects.** Outbox publication and receiver-side effect deduplication close different gaps. **Established.** [Transactional outbox][s32]; [Restate external effects][s30].
11. **P11 — Time belongs in the protocol.** Define expiry, late results, and who decides when completion races with a deadline. **Established basis; review rule is synthesis.** [IoT query example][s18]; [OTP state machines][s05].
12. **P12 — Bound responsibility over time.** State, pending work, retained outcomes, and subscriptions need limits and exit paths. **Established concern; limits are domain-specific.** [Erlang in Anger][s07].
13. **P13 — Use only the coordination the problem needs.** Functions, transactions, streams, and batch tools remain appropriate. Actors need a concrete justification. **Synthesis supported by suitability guidance.** [Dapr overview][s22]; [Cloudflare rules][s24]; [CQRS caution][s34].
14. **P14 — Verify adverse protocol traces.** Isolation does not remove message races, stale assumptions, or logical deadlocks. **Established empirically.** [Bagherzadeh et al.][s49]; [Torres Lopez et al.][s50].
15. **P15 — Preserve promises during evolution.** Splits, merges, and new protocols must account for old identities and unfinished work. **Synthesis.** [Vernon II][s09]; [Rebeca design method][s53].

### State the assumptions that change a design

Do not import a framework's semantics into a technology-agnostic model:

- **If delivery can lose messages**, choose acknowledgement, retry, expiry, or explicitly accepted loss.
- **If delivery can repeat messages**, protect non-repeatable changes using logical operation identity.
- **If a request excludes others until completion**, a slow dependency can prevent all later requests from progressing.
- **If requests interleave across an await**, revalidate earlier observations or establish a reservation first. Single-threaded execution does not make an asynchronous operation indivisible. [Orleans scheduling][s21]; [Cloudflare rules][s24].
- **If state, input acknowledgement, and outgoing intent commit together**, name that guarantee. Otherwise, model each gap.
- **If instance loss discards memory**, identify what can be reconstructed and which promises require persistence.
- **If reminders survive instance loss**, use their documented delivery contract. Do not assume an ordinary timer has that property. [Dapr timers][s23].
- **If cross-owner transactions or durable locks exist**, specify their scope. Neither implies rollback of arbitrary external effects. [Orleans transactions][s61]; [Azure entities][s26].

## 2. Design questions, in modelling order

This method is **synthesis**. EventStorming discovers the narrative; aggregate design discovers consistency constraints; state-machine and protocol techniques make responsibilities precise. None mechanically determines the actors. [EventStorming][s54]; [Vernon I][s08]; [P][s52].

### Step 1: establish the promise

1. What outcome is the user buying? Describe success, refusal, cancellation, and uncertainty.
2. What must never happen? What must eventually happen, under which availability assumptions?
3. Which effects are irreversible, expensive to repeat, human-visible, or time-sensitive?
4. Is the problem independent stateful activity, a long conversation, a pure transformation, a bulk query, or a tightly coupled transaction?
5. What simpler non-actor design would work? What specific difficulty would actors remove?

**Produce:** a short contract and explicit constraints.

### Step 2: discover participants and facts

6. What facts occur? Which command or outside occurrence causes each one?
7. Who acts independently: customers, devices, sessions, jobs, auctions, or business processes?
8. Which facts trigger policies, human decisions, or future work?
9. Where do meanings differ? A booking's payment status is not the payment provider's ledger.
10. What knowledge is authoritative, derived, cached, or suspected?

**Produce:** a domain timeline. A human “actor” in EventStorming is not automatically a software actor.

### Step 3: locate consistency owners

11. Which data and decision must be considered together for each invariant?
12. Can one proposed owner reject conflicting commands without consulting stale information elsewhere?
13. If a rule crosses owners, can reservations, allocated rights, or eventual completion satisfy the business?
14. If not, should the boundary grow or should this part use a transaction?
15. Which entities and values have no independent authority or lifecycle and should remain inside an owner?

**Produce:** an invariant-to-owner map. Every rule needs an owner or a precise coordination protocol.

### Step 4: choose identity and lifecycle

16. What makes two requests address the same actor? Specify tenant, kind, key, normalization, and namespace.
17. Is identity an enduring entity, a conversation, or a resource incarnation?
18. Can a key be reused? What happens to old messages after deletion?
19. Who initializes the entity? What happens on duplicate or concurrent creation?
20. Who closes it? What happens to children, subscriptions, pending work, and late replies?
21. Are business containment, creation, and failure responsibility actually the same relationship?

**Produce:** identity rules and a lifecycle sketch. Business closure must not depend on whether an in-memory instance happens to exist.

### Step 5: write responsibility cards

22. What sentence describes each actor's purpose?
23. Which decisions and mutable facts belong exclusively to it?
24. What does it know only by observing others?
25. What does it not own?
26. Why does it depend on each collaborator? Could immutable input remove a dependency?
27. Which collections grow over time, and who expires or summarizes them?

**Card fields:** identity; purpose; authority; invariants; public protocol; emitted facts; collaborators; lifecycle; durable promises; effects; deadlines; bounds; recovery owner.

### Step 6: specify protocols

28. Is each message a command, event, query, or reply?
29. Who owns its meaning and compatibility? Which messages are internal?
30. What identifies the business operation, conversation, attempt, reply destination, precondition, and expiry?
31. What does each acknowledgement prove: received, accepted, durably recorded, reserved, submitted, or completed?
32. Can a recipient decide from its own state and the message? Who owns any needed conversation?
33. What happens on duplicates, conflicting key reuse, invalid transitions, stale replies, and old versions?
34. Is a reply needed, or can forward flow or a later status query suffice?

**Produce:** a message catalogue and state/event table.

### Step 7: draw coordination and time

35. Draw success, timeout, duplicate delivery, reorder, cancellation, and dependency failure.
36. What must be sequential? What is independent? What completes fan-in?
37. Does anyone wait while preventing the callback or peer decision needed to finish?
38. Who records process progress and selects recovery? Why orchestration or choreography?
39. What can be reserved? How do confirm, cancel, renew, and expiry interact?
40. Who decides a deadline? What if its notification is late, repeated, or obsolete?
41. What if a human never responds or approves an obsolete proposal?

**Produce:** sequence diagrams with failure branches and deadline rules.

### Step 8: define external boundaries

42. Who authorizes an external action, and who translates it?
43. What identifies the effect across retries and restarts?
44. Can the recipient deduplicate, query status, reject stale writers, or support conditional writes?
45. If an effect happens before its result is recorded, what is the next safe action?
46. Which inbound events can repeat, arrive out of order, or be missed? How can gaps be repaired?
47. Who owns a connection or child process, and who owns the lasting business resource?
48. What happens during outages, exhausted quotas, and open circuits?

**Produce:** an effect ledger, gateway contract, and reconciliation plan.

### Step 9: verify and simplify

49. Can a short trace violate an invariant or leave accepted work pending without an accountable owner?
50. Which properties need generated traces? Which protocol merits model checking?
51. Which actors merely forward calls or expose fields and could become functions or be merged?
52. Which actor mixes unrelated decisions or lifetimes and could be split?
53. How do old callers, timers, and conversations interact with the new design?
54. Can someone review the promises from the model without reverse-engineering every handler?

**Produce:** tested assumptions, counterexample traces, and an evolution plan.

## 3. Pattern catalogue

Each entry states **problem; solution; trade-offs; avoid; sources**. Concrete applications and trade-offs are synthesis.


### Choosing granularity and deciding against actors

**Synthesis.** A candidate earns an actor boundary when it has an independent decision, lifetime, conversation, resource, or recovery obligation. Start with the smallest boundary that can uphold its immediate invariants. Then examine the actual collaborations.

**Too coarse:** unrelated entities must share one decision point; unrelated work shares failure or cancellation; one participant holds several incompatible lifecycles; routine changes require understanding every domain. Split only after naming the authority and promises each new participant receives.

**Too fine:** a business operation crosses boundaries merely to read fields; most actors have no decisions; participants always start and end together; every update needs reciprocal agreement. Merge tightly coupled responsibility or use local values and functions.

Prefer another model when the problem is primarily a pure transformation, bulk analytics, relational reporting, or a small CRUD application with adequate transactional rules. If almost every operation requires one indivisible change across proposed actors, reconsider the boundaries or use an appropriate transaction system. A graph of actors is not a substitute for deciding which operations must be atomic. Large independent compute jobs can still justify workers; the objection is to turning each arithmetic operation into a protocol. [Orleans suitability][s14]; [Dapr suitability][s22]; [Cloudflare rules][s24].

For a mixed system, apply actors only where their ownership and conversation model helps. A projection database, functional domain core, transactional subsystem, and actor-controlled external resource can coexist. Document where authority changes; do not silently bypass an actor's invariant through the shared infrastructure.

### A. Decomposition

**A1 — Entity or aggregate actor.** **Problem:** concurrent commands concern one business identity and its rules. **Solution:** one logical decision owner per consistency boundary; dependent values stay inside it. **Trade-off:** strong local authority but concentrated responsibility for a popular identity. **Avoid:** one actor per table row or splitting an inseparable invariant. [Vernon I][s08]; [Orleans practices][s14].

**A2 — Actor per independent activity.** **Problem:** calls, sessions, matches, or jobs progress independently. **Solution:** each gets a protocol, identity, and completion condition. **Trade-off:** natural cancellation and fault boundaries, with explicit cleanup. **Avoid:** converting every noun or function into an activity. [Armstrong, Concurrency Oriented Programming][s62].

**A3 — Resource owner or digital twin.** **Problem:** several clients interact with one device, connection, account, or external process. **Solution:** one owner governs permitted actions and records observations; distinguish resource identity from connection incarnation. **Trade-off:** centralized policy, but observations can lag reality. **Avoid:** claiming exclusive physical authority when other systems can act too. [IoT registration][s17]; [Erlang ports][s27].

**A4 — Conversation or process actor.** **Problem:** a request needs several replies, decisions, or deadlines. **Solution:** a conversation identity owns pending state and completion; use persistence only if the promise requires survival. **Trade-off:** isolates complexity but adds a participant. **Avoid:** a durable process for a trivial local calculation. [Akka interactions][s16]; [Process Manager][s37].

**A5 — Responsibility hierarchy.** **Problem:** components depend on others being restored correctly. **Solution:** draw a failure-responsibility tree distinct from the communication graph; choose independent or grouped recovery. **Trade-off:** accountability versus collateral failure from broad groups. **Avoid:** copying business containment or organization charts without analysing dependencies. [OTP supervision][s04]; [Proto.Actor][s25].

**A6 — Pure core within an actor.** **Problem:** parsing, validation, or calculation becomes unnecessary messaging. **Solution:** use functions and values inside the owner; separate workers only for independent jobs, isolation, cancellation, or resources. **Trade-off:** simpler protocols; execution remains the caller's responsibility. **Avoid:** denying a genuinely independent compute job an appropriate worker boundary. [Cloudflare rules][s24]; [Scala mixing study][s48].

### B. State and reads

**B1 — Explicit state machine.** **Problem:** flags and callbacks permit impossible combinations. **Solution:** state/event tables specify next state, response, effects, and invalid-input policy. Behaviours, FSM facilities, and plain data can express this. **Trade-off:** explicit modelling work. **Avoid:** naming every possible data value as a distinct control state. [OTP gen_statem][s05].

**B2 — Pure decision plus effect plan.** **Problem:** domain rules disappear among dependency calls. **Solution:** model `decide(state, input) -> state + intended actions`; feed action outcomes back as inputs. Define the durable boundary for state and intent. **Trade-off:** easier testing; this split does not create atomic external effects. **Avoid:** elaborate effect infrastructure for simple handlers. **Synthesis** informed by [P][s52] and [outbox][s32].

**B3 — Query the authority.** **Problem:** a caller needs one owner's current information. **Solution:** return a domain-shaped snapshot or decision, with a version if later action depends on it. Prefer `TryReserve` over `GetAvailability` then a blind update. **Trade-off:** the answer can become stale immediately. **Avoid:** treating several queries as a global snapshot. [IoT query][s18]; [Vernon II][s09].

**B4 — Projection / read model.** **Problem:** search and reporting cross ownership boundaries. **Solution:** maintain a derived query model through an explicit publication contract; state freshness, rebuild, and correction rules. **Trade-off:** useful views but additional consistency work. **Avoid:** enforcing scarce-resource allocation from a stale projection or adding CQRS to simple CRUD reflexively. [Fowler, CQRS][s34].

**B5 — Bounded current state.** **Problem:** historical events, subscribers, results, and pending operations grow forever. **Solution:** separate present decision state from history and blobs; specify retention and preserve necessary retry/deletion evidence. **Trade-off:** retention affects observable behaviour. **Avoid:** discarding deduplication records before the replay horizon ends. [Erlang in Anger][s07]; [Claim Check][s39]; [Stripe retention][s33].

### C. Protocols

**C1 — Commands express intent; events express facts.** **Problem:** generic updates put rules in callers. **Solution:** `ReserveSeat` requests action; `SeatReserved` reports a fact; `GetReservation` queries; `ReservationRejected` replies. A reply need not be broadcast. **Trade-off:** stronger meaning requires vocabulary agreement. **Avoid:** disguising mandatory work as an event with no responsible executor. [Command Message][s63]; [Event Message][s64].

**C2 — Correlated asynchronous request/reply.** **Problem:** replies overlap and arrive late. **Solution:** explicit conversation/request ID and reply destination; a future can package this interaction. **Trade-off:** pending state and timeout policy. **Avoid:** requiring replies to notifications whose outcomes nobody needs. [Correlation Identifier][s41]; [Return Address][s65].

**C3 — Owned, typed vocabulary.** **Problem:** universal message bags expose internals. **Solution:** receiver-owned command meaning, publisher-owned fact meaning, and agreed integration contracts; private internal completion/timer messages. **Trade-off:** payload types do not prove valid ordering. **Avoid:** unrestricted “anything” protocols. Ownership conventions are **synthesis**; [Pekko style][s20] supports explicit public/private messages and [session actors][s51] addresses sequencing.

**C4 — Protocol state machine / session type.** **Problem:** valid message types appear in invalid phases or roles. **Solution:** describe the whole conversation, including failure branches; use session types or monitors where useful. **Trade-off:** stronger analysis under explicit assumptions. **Avoid:** equating typed references with liveness or recovery guarantees. [Fowler, session actors][s51].

**C5 — Idempotent business command.** **Problem:** retries repeat a change. **Solution:** stable logical key bound to canonical intent; duplicate returns established result or current status; conflicting reuse is rejected. Atomically retain deduplication evidence with the protected local update where possible. **Trade-off:** retention and key scope are contractual. **Avoid:** new keys per attempt or deduplication by payload equality alone. [Idempotent Receiver][s40]; [Helland][s10].

**C6 — Self-contained message / claim check.** **Problem:** recipient needs repeated callbacks or huge payloads. **Solution:** include intent, required immutable facts, and versions; reference immutable large content with explicit access and retention. **Trade-off:** references create lifetime dependencies. **Avoid:** interpreting a mutable row's latest contents as the original request. [Claim Check][s39].

**C7 — Forward flow / continuation.** **Problem:** A waits for B waits for C, though intermediaries do not use results. **Solution:** pass next or final destination and forward work; retain an intermediary only for a real decision or recovery obligation. **Trade-off:** fewer waits, more explicit routing. **Avoid:** bypassing required validation or publication authority. [Pipes and Filters][s42]; [Return Address][s65].

**C8 — Semantic protocol evolution.** **Problem:** messages and conversations outlive deployments. **Solution:** preserve meanings for old callers; version changed conversation rules and translate at boundaries. **Trade-off:** compatibility overlap and migration work. **Avoid:** treating payload compatibility as proof of behavioural compatibility. **Synthesis** informed by [Orleans versioning][s66] and [session actors][s51].

### D. Coordination

**D1 — Entity-local partner knowledge.** **Problem:** owners cooperate without one atomic update. **Solution:** record requested, promised, confirmed, cancelled, and unknown states per relationship. **Trade-off:** explicit intermediate states and protocol work. **Avoid:** treating a previous reply as a lock on the partner. [Helland activities][s10]. His “activity” is relationship state, not necessarily another actor or a Temporal Activity.

**D2 — Saga.** **Problem:** business work spans independently committed steps. **Solution:** describe forward steps and semantic compensations; record progress and recover forward or compensate. **Trade-off:** partial effects are visible and compensation can fail. **Avoid:** promising all-or-nothing visibility or reversible physical history. [Sagas][s11]; [McCaffrey][s12]; [Compensating Transaction][s35].

**D3 — Per-process orchestration.** **Problem:** branching and recovery are hard to reconstruct from distributed reactions. **Solution:** one process identity selects the next step and records obligations; entities keep their local business rules. **Trade-off:** clarity versus excessive central responsibility. **Avoid:** one global coordinator for unrelated orders. [Process Manager][s37].

**D4 — Choreography.** **Problem:** autonomous capabilities react to shared facts. **Solution:** publish facts; subscribers own reactions; document end-to-end completion and exception ownership. **Trade-off:** autonomy versus hidden causal dependencies. **Avoid:** long mandatory processes nobody can diagnose or repair. [Publish-Subscribe][s43]; [Saga comparison][s67].

**D5 — Reservation / try-confirm-cancel.** **Problem:** compensation would be expensive or impossible. **Solution:** reserve provisionally, then confirm or cancel; owner tracks token, expiry, renewal, and terminal transitions. **Trade-off:** withholds capacity and still needs recovery. **Avoid:** assuming independent expiring holds guarantee atomic group commitment. [Tentative Operation][s38].

**D6 — Allocated rights / escrow.** **Problem:** independent owners must respect a shared upper bound. **Solution:** allocate non-overlapping rights; each owner spends only its allocation; transfers preserve total rights. **Trade-off:** local progress but stranded capacity. **Avoid:** merely convergent counters when temporary overspending is forbidden. [Bounded counters][s68]; actor realization is **synthesis**.

**D7 — Aggregator / fan-in.** **Problem:** related replies must form one result. **Solution:** define correlation, membership, completeness, and combination; choose all, threshold, first acceptable, deadline, or external event. **Trade-off:** partial-result policy is necessary. **Avoid:** waiting forever for an unspecified number of responses. [Aggregator][s36].

**D8 — Scatter-gather / fan-out.** **Problem:** independent contributors can answer in parallel. **Solution:** choose participant set, distribute, collect under a deadline; distinguish absent data, refusal, unavailable, and timeout. **Trade-off:** multiplied work and late results. **Avoid:** speculative duplicate commands with irreversible effects. [Scatter-Gather][s69]; [IoT query][s18].

**D9 — Pipeline / routing slip.** **Problem:** independent stages vary by request. **Solution:** fixed pipeline or approved route carried with the message; stable step identity and exception destination. **Trade-off:** cancellation and partial completion need propagation. **Avoid:** linear routing slips for branching based on future results. [Pipes and Filters][s42]; [Routing Slip][s44].

**D10 — Publish-subscribe.** **Problem:** several independent reactions need one fact. **Solution:** define subscriptions, replay, duplicates, and subscriber lifetimes. **Trade-off:** publisher need not know everyone; publication is not completion of all reactions. **Avoid:** broadcast when one accountable executor is required. [Publish-Subscribe][s43].

**D11 — Registry / index.** **Problem:** actors need discovery or membership information. **Solution:** distinguish lookup, derived membership, and authoritative uniqueness; route directly after discovery when appropriate. **Trade-off:** indirection versus centralized dependency. **Avoid:** deciding hard uniqueness from a stale index. [IoT registration][s17]; [Orleans practices][s14].

**D12 — Work distribution with admission.** **Problem:** independent jobs need execution without unlimited obligations. **Solution:** define job IDs, acceptance, assignment, completion, abandonment, retry, and cancellation; bound accepted work. **Trade-off:** execution can overlap after failure. **Avoid:** confusing an interchangeable worker with the authoritative owner of an invariant. [Competing Consumers][s45]; [Erlang in Anger][s07].

### E. Time

**E1 — Deadline transition.** **Problem:** waiting needs a business outcome. **Solution:** retain deadline and conversation generation; timeout evaluates current state; late completion has a stated treatment. **Trade-off:** needs clock and delay assumptions. **Avoid:** equating caller timeout with cancellation. [IoT query][s18]; [OTP state machines][s05].

**E2 — Durable reminder.** **Problem:** an obligation survives a long wait or restart. **Solution:** retain obligation and due time, use a scheduler with matching guarantees, recognize duplicate and obsolete firings. **Trade-off:** durable scheduling is not an exact execution-time promise. **Avoid:** volatile timers for durable business obligations. [Dapr timers][s23].

**E3 — Debounce / throttle / batch.** **Problem:** arrival frequency exceeds useful decision frequency. **Solution:** debounce replaceable updates with a maximum wait; throttle by explicit admission policy; batch by count, size, or deadline and retain item identities. **Trade-off:** deliberately changes latency or which inputs survive. **Avoid:** dropping ledger commands under a policy for replaceable observations. **Synthesis** informed by [Aggregator][s36] and [overload design][s07].

**E4 — Missing-event detector.** **Problem:** an expected event never arrives. **Solution:** track last observation and generation; mark suspect after a deadline; define what suspicion allows. **Trade-off:** delay may resemble failure. **Avoid:** assuming a missed heartbeat proves an old participant cannot act. [Hebert, Zen][s63a]; [IoT query][s18].

**E5 — Human task.** **Problem:** approval may take days. **Solution:** durable task identity, assignee, proposal version, expiry, and terminal outcome; commands for approve, reject, withdraw, or reassign. **Trade-off:** stale decisions and authority require validation. **Avoid:** tying the task to a connection or holding unrelated decisions hostage. [Temporal messages][s29]; [Azure human interaction][s70].

### F. Failure

**F1 — Business failure as data.** **Problem:** normal refusal looks like broken execution. **Solution:** explicit refusal, conflict, and expiry outcomes; say which allow revised requests. **Trade-off:** outcome algebra must still preserve uncertainty. **Avoid:** crashing to retry invalid input. [Akka introduction][s15].

**F2 — Error kernel and risky leaves.** **Problem:** unstable work threatens valuable state. **Solution:** small trusted decision core, independent risky workers, validated results. **Trade-off:** isolation does not reverse prior effects. **Avoid:** irreplaceable state held only in disposable workers. [Armstrong appendix B][s03]; [Hebert, Zen][s63a].

**F3 — Recovery contract and escalation.** **Problem:** “restart” has no specified meaning. **Solution:** say what survives, what is reconstructed, how promises resolve, and when failure escalates. **Trade-off:** recovery groups determine collateral impact. **Avoid:** catching faults and continuing with untrusted state. [OTP supervision][s04]; [Proto.Actor][s25].

**F4 — Retry owner and budget.** **Problem:** layers retry independently or forever. **Solution:** choose the owner that knows logical identity, deadline, and effect safety; bound attempts or elapsed time. **Trade-off:** recovery attempts consume resources. **Avoid:** blind retry of uncertain irreversible work. [Temporal design patterns][s71].

**F5 — Quarantine / parking lot.** **Problem:** an input repeatedly fails. **Solution:** retain identity, input, error evidence, version, and status; assign repair/replay authority; decide whether later same-entity work can proceed. **Trade-off:** parking creates an obligation, not success. **Avoid:** treating runtime dead letters as a durable recovery queue. [Dead Letter Channel][s46]; [Proto.Actor actors][s72].

### G. External boundaries

**G1 — Gateway / adapter / anti-corruption boundary.** **Problem:** transport and vendor semantics leak inward. **Solution:** translate identities, units, operations, and errors; make the boundary an actor when it owns meaningful state. **Trade-off:** semantic isolation versus gateway overreach. **Avoid:** an actor for every stateless wrapper. [Messaging Gateway][s47].

**G2 — Effect ledger and reconciliation.** **Problem:** durable intent and external reality disagree. **Solution:** retain intended operation, attempts, evidence, and outcome; resolve ambiguity through the external contract. **Trade-off:** some effects cannot be resolved automatically. **Avoid:** a lone `sent` Boolean. [Temporal activities][s28]; [Restate][s30]; [Quicksand][s73].

**G3 — Transactional outbox / idempotent inbox.** **Problem:** state and notification separate on failure. **Solution:** record state and outgoing intent atomically; deliver later; atomically deduplicate with the receiver's local update where supported. **Trade-off:** delivery can repeat. **Avoid:** claiming arbitrary external effects are exactly once. [Outbox][s32]; [Idempotent Receiver][s40].

**G4 — Quota owner and circuit state.** **Problem:** callers share limited capacity or an unhealthy dependency. **Solution:** scope admission to the actual quota; model circuit state and controlled probes; reject, defer, or degrade explicitly. **Trade-off:** fairness and concentrated authority. **Avoid:** assuming per-user limits enforce a provider-wide quota. [Circuit Breaker][s31]; [Halo incident][s57].

### H. Worked design exercise: seats, payment, and tickets

This is **original synthesis**, not a framework sample.

A customer reserves a seat, pays an external processor, and receives a ticket. The hard local rule is that one seat has at most one confirmed booking. Payment and ticket issuance cannot be promised to change simultaneously when they belong to independent systems.

| Participant | Identity and owned knowledge | Responsibility |
| --- | --- | --- |
| SeatInventory | event edition + seat | Available capacity, current reservation token, expiry, confirmed booking. |
| BookingProcess | tenant + booking ID | Requested seats, offer version, process progress, cancellation intent, remaining obligations. |
| PaymentOperation | merchant account + logical payment ID | Charge intent, attempts, provider evidence, known outcome; not the provider's ledger. |
| PaymentGateway | provider account or suitable connection scope | Translation, shared quota, dependency health when these need stateful coordination. |
| TicketIssue | booking + issue version | Logical ticket issuance and immutable artifact reference. Merge into BookingProcess unless it needs independent ownership. |
| BookingReadModel | query-oriented records | Derived UI/search view. It cannot allocate seats. |

Money, seat labels, and pure price calculations remain values or functions. There is no global BookingManager through which every step must pass.

Example messages include `ReserveSeat(reservationId, bookingId, offerVersion, expiresAt, replyTo)`, `SeatReserved`, `SeatUnavailable`, `AuthorizePayment(paymentId, amount, currency)`, `PaymentConfirmed(paymentId, providerReference)`, `PaymentDeclined`, `PaymentOutcomeUnknown`, and `ConfirmSeat(reservationId, bookingId)`.

The seat owner resolves confirm-versus-expiry locally. A delayed confirmation cannot revive an unrelated or expired hold. If payment succeeds after the seat becomes unavailable, the process executes the agreed policy: refund, replacement offer, or human resolution.

| Payment state | Input | Decision |
| --- | --- | --- |
| NotRequested | New payment intention | Validate and retain intent; arrange submission. |
| SubmissionPending | Duplicate same ID and intent | Return current status; do not create another logical charge. |
| SubmissionPending | Timeout or lost response | Mark Unknown; start reconciliation. |
| Unknown | Authenticated confirmation | Record Confirmed; notify the process. |
| Unknown | Another timeout | Remain Unknown; follow section 6. |
| Confirmed | Duplicate confirmation | Preserve the outcome. |
| Confirmed | Approved cancellation | Create a separately identified refund obligation. |
| Any retained state | Same key, conflicting amount or currency | Reject key misuse. |

Per-seat owners prevent double sale of individual seats. They do not provide atomic group booking. A process can release partial reservations after failure if the business accepts that behaviour. If the group must appear atomically allocated, change the boundary or use a transaction with exactly that scope.

The model still needs decisions about how long reservation uncertainty is tolerated, whether payment is authorized before capture, and when a ticket becomes usable. These are product questions, not runtime configuration.

## 4. Do's and don'ts

The following are compact **synthesis rules**, tied to the principles and patterns above.

### Decomposition and identity

- Do name each actor's decision authority; a noun alone does not justify concurrency. P01/A1.
- Do choose granularity from invariants, lifecycle, and independent work; line counts are weak evidence. P02/A2.
- Do separate lasting resource IDs from connection or session IDs; reconnecting must not create a new business entity accidentally. A3.
- Do specify tenant and key normalization; inconsistent addressing can create two apparent owners. P02/P15.
- Don't split a hard invariant to reduce actor size without replacing its enforcement mechanism. P02/D6.
- Don't make every helper a child actor; functions have simpler contracts. P13/A6.
- Don't reject all singletons: a genuinely singular, low-volume authority can be appropriate; justify its scope. A1/D11.

### State and reads

- Do distinguish authoritative facts, observations, and projections; their freshness and authority differ. P04/B3/B4.
- Do define invalid state/message combinations; silent acceptance conceals protocol errors. P03/B1.
- Do bound histories, subscribers, and pending work; an actor can leak obligations as well as memory. P12/B5.
- Don't authorize a later mutation solely from a previous query; intervening changes may invalidate it. P02/B3.
- Don't let another service modify actor-owned records directly; shared storage is not shared authority. P04.
- Don't add event sourcing or CQRS merely because actors are present; each needs a separate requirement. P13/B4.

### Protocols and coordination

- Do use domain verbs and explicit outcomes; callers need intent and guarantees. P03/C1.
- Do keep operation, conversation, and attempt identities conceptually distinct; they solve different problems. P09/C2/C5.
- Do include enough immutable context to decide; avoid needless conversational dependencies. C6.
- Do use tell-don't-ask to place decisions with their owner; queries remain valid for information. B3/C1.
- Don't turn every interaction into an ask; each reply creates an obligation to correlate, expire, and recover. C2/C7.
- Don't call choreography “decoupled” without documenting causal dependencies and completion ownership. D4.
- Don't treat a saga as isolation or compensation as time travel. D2.
- Don't solve call cycles by enabling interleaving without rechecking invariants across awaits. P14.

### Time and failure

- Do distinguish a deadline from a timer notification; notification delivery can be late. P11/E1.
- Do reject obsolete timer generations; an old timeout must not end a new conversation. E1.
- Do distinguish expected refusal, transient inability, and unknown outcome; recovery differs. P06/F1/F4.
- Do specify what restart means for promises and state; recreating an instance is not completing its work. P07/F3.
- Don't retry independently at every layer; retries multiply and lose business context. F4.
- Don't park work without an owner and replay rules; a parking lot can become silent data loss. F5.

### Outside world and verification

- Do preserve one effect identity across uncertain retries; changing it defeats receiver deduplication. P09/G2.
- Do treat “accepted” and “completed” as separate claims when the dependency does. P08.
- Do reconcile desired and observed state; an actor cannot make external reality obey by remembering a value. G2.
- Don't assume closing an actor undoes a payment, kills every descendant program, or retracts an email. P10.
- Do test duplicates, timeouts, late success, and crash gaps; happy-path examples miss protocol errors. P14.
- Don't call a type-safe design deadlock-free without examining its conversations. C4/P14.

## 5. Anti-patterns: symptoms, harm, and repairs

These diagnostics are **synthesis**. Their evidence base includes suitability guidance, messaging patterns, and empirical bug studies. A symptom calls for investigation; it is not proof that a design is wrong.

| ID and anti-pattern | Symptoms in a design or code | Why it hurts | Repair and basis |
| --- | --- | --- | --- |
| AP01 God actor | Handles unrelated entities, policies, routing, retries, and reporting; changes for every feature. | Expands failure scope and makes promises hard to isolate. | Split by independent decisions/lifetimes; keep immediate invariants together. P01/P02, A1/A5. |
| AP02 Anaemic CRUD actors | Getters/setters dominate; callers read, calculate, and write back. | Rules live outside the state owner; check-then-act races remain. | Put domain commands and validation at the owner, or use ordinary CRUD if there is no domain behaviour. B3/C1. |
| AP03 Actors used as synchronous objects | Property-like remote calls, blocking waits, hidden callbacks, expectations of immediate exceptions. | Distribution and failure semantics disappear behind object syntax. | Explicit coarse operations, outcome contracts, and asynchronous progress. [Orleans practices][s14]. |
| AP04 Ask chains everywhere | A waits on B, B on C; several layers add their own timeout/retry. | Dependency failures propagate, and cancellation becomes ambiguous. | Forward flow, per-conversation state, or a deliberate orchestrator. C2/C7/D3. |
| AP05 Chatty protocols | Many round trips for one decision; repeated retrieval of tiny fields. | Adds failure windows and moves decisions away from their owner. | Coarse intent commands, self-contained input, or merge tightly coupled owners. A1/C6. |
| AP06 Too-fine granularity | Actors only proxy functions; lifetimes always coincide; no independent invariant. | Protocol bookkeeping exceeds useful responsibility. | Use values/functions or merge. A6. |
| AP07 Shared-state side channel | Two actors update the same record; admin scripts bypass rules; callbacks mutate owner memory. | No single authority remains; encapsulation is illusory. | Assign mutation ownership; route repairs through a controlled owner protocol or explicitly change the consistency model. [Scala mixing study][s48]. |
| AP08 Singleton bottleneck | Every unrelated operation visits one manager, registry, or quota actor. | Concentrates dependencies and recovery scope. | Partition by real business authority; staged aggregation; direct routing after lookup. D11/D12. |
| AP09 Split invariant | Two actors independently approve changes that jointly violate a bound. | Local correctness does not compose into the promised global rule. | Merge authority, allocate rights, reserve, or use an appropriately scoped transaction. P02/D5/D6. |
| AP10 Mutable message alias | Sender changes an object after sending; receiver returns an internal mutable collection. | Shared-memory races re-enter through the API. | Immutable values, copies, or provable ownership transfer supported by the language. P04. |
| AP11 Cyclic waiting | A awaits B while B needs A; actors are alive but the business operation never moves. | Logical deadlock can exist without thread locks. | Remove waiting cycles, use continuations or process state; prove safe use of interleaving. [Orleans scheduling][s21]; [Torres Lopez][s50]. |
| AP12 Hidden order/timing assumption | Sleep “fixes” a test; events from different sources are presumed ordered; late replies update current work. | Timing changes alter correctness. | Explicit versions, causality, generations, and invalid-transition treatment. [Actor bug study][s49]. |
| AP13 Unbounded state | Ever-growing completed IDs, subscriber maps, histories, or pending requests. | Recovery and responsibility grow without limit. | Retention contract, summaries, archival, admission bounds. B5/P12. |
| AP14 Actors for pure computation | A chain of actors performs trivial transformations with no independent ownership. | Creates unnecessary distributed control flow. | Pure functions or dataflow tools; keep workers only where justified. A6/P13. |
| AP15 Crash-as-business-logic | Expected “not found” or “insufficient funds” crashes and restarts. | Retry loops replace useful outcomes. | Typed/domain failures; reserve fault handling for broken execution assumptions. F1. |
| AP16 Restart equals durability | Success is acknowledged before important state is safe; recovery returns empty state. | A living replacement violates a previous promise. | Define durable acknowledgement and reconstruction requirements. P07/F3. |
| AP17 Exactly-once wish | “Mark sent then call” or “call then mark done” with no ambiguity state. | One ordering loses effects; the other can duplicate them. | Effect ledger, receiver deduplication, reconciliation, or explicit human handling. Section 6. |
| AP18 Outbox overclaim | Outbox delivery is used to justify exactly-once payments or emails. | It only protects the local state/publication boundary. | Analyse recipient-side atomicity and effect semantics separately. G3. |
| AP19 Timeout equals failure | Timeout immediately frees a resource or triggers compensation while the original action continues. | Late success conflicts with cancellation or replacement. | Unknown state; cancellation protocol; fencing or reconciliation where supported. E1/G2. |
| AP20 Poison-message restart loop | Same input repeatedly crashes a valuable owner; replay has no terminal handling. | Progress is impossible and good work can be blocked. | Quarantine or deliberate fail-stop with operator ownership; never skip a critical event silently. F5. |
| AP21 Dependency availability is initialization | A remote outage prevents a recoverable client actor from reaching a stable state. | External failure becomes local boot/restart failure. | Model Disconnected as a valid service state when the business permits it. [Hebert, Guarantees][s06]. |
| AP22 Digital-twin omniscience | Stored desired state is presented as measured reality; reconnect resets device history. | The model lies about physical state and lifecycle. | Separate desired, reported, observed-at, and confidence; reconcile by incarnation. A3/G2. |

A shared SQL database is not automatically AP07. Actors can share infrastructure while owning disjoint state. A projection may read committed facts through an agreed interface. The smell is uncontrolled mutation or reliance on storage internals that bypasses the owner's contract.

A synchronous-looking asynchronous API is not automatically AP03. Orleans intentionally uses this style. The defect is reasoning about it as an infallible local object call, not the syntax itself.

## 6. Actors and the outside world

This section gives the boundary design extra attention. The detailed decision procedure and examples are **synthesis**. Its central premise is directly supported by Temporal and Restate: a side effect can complete before the caller durably records its result. [Temporal][s28]; [Restate][s30].

### 6.1 Separate four kinds of authority

For each external integration, identify:

1. **Business authority:** who may decide that an action should happen?
2. **Protocol adapter:** who translates that intention into the external system's vocabulary?
3. **External authority:** which system establishes whether the action actually happened?
4. **Recovery authority:** who resolves ambiguous or conflicting evidence?

These may be functions within one component or separate actors. Splitting them into four actors is not required.

For example, a booking decides whether to request a charge. A payment adapter knows the provider API. The provider's ledger establishes whether money moved. A payment operation or reconciliation process owns unresolved outcomes. The adapter must not invent a business refund policy merely because it sees a network error.

A gateway should normalize error categories, identities, units, and status semantics. Keep vendor-specific concepts at that boundary unless they are meaningful business facts. A stateless translator can remain a function. State such as connection ownership, shared rate allowance, in-flight command IDs, or circuit health can justify an actor. [EIP Messaging Gateway][s47].

### 6.2 Write an external-effect contract

For every effect, record:

| Field | Question |
| --- | --- |
| Logical identity | What durable key means “this exact intended effect”? |
| Intent | What canonical parameters and target does the key bind to? |
| Authority | Who authorized it, and under which business version? |
| Recipient semantics | Idempotent operation, idempotency key, conditional write, reservation, or none? |
| Status evidence | What response, query, event, receipt, or observation proves success? |
| Negative evidence | Can “not found” really prove absence, or can the request still commit later? |
| Retention | How long does the recipient remember keys and outcomes? |
| Concurrency | Could an older attempt remain active? Can the recipient reject stale generations? |
| Reversibility | Is compensation available? Does it restore value, state, or only some business acceptability? |
| Completion promise | Submitted, accepted, settled, observed, or verified? |
| Recovery owner | Who checks and resolves Unknown, by what deadline? |
| Cancellation | Can it prevent execution, request reversal, or merely stop waiting? |

A strong local record might contain operation ID, intent digest, target identity, phase, attempt metadata, external reference, last evidence, next reconciliation time, and required human action. These are suggested fields, not a required schema.

Useful phases include **Planned**, **Ready**, **Submitted/Unknown**, **Confirmed**, **DefinitivelyRejected**, **CompensationRequired**, **Compensated**, and **NeedsReview**. “Submitted” may be evidence of an attempt, not evidence of remote acceptance.

### 6.3 The failure gap

Consider a non-idempotent external operation:

1. The actor retains its intention.
2. It invokes the dependency.
3. The dependency performs the effect.
4. The actor records the result.

A failure between 3 and 4 leaves the same local evidence as some failures before 3. Reversing the recording order does not fix this: recording success before the call can lose an effect instead.

A durable execution journal can preserve a completed step's result for replay. An unfinished step can still run again. A pure actor's private state cannot close a transaction in an unrelated external service. [Restate][s30]; [Temporal][s28].

**Design consequence:** Unknown is a real outcome. It needs an owner and a recovery path. It must not be silently collapsed into Failed or Succeeded.

### 6.4 Decision guide: the effect may have happened, but its result is missing

Apply these questions in order. Check the actual recipient contract; do not infer it from an HTTP method name alone.

| Condition | Safe design action | What must be true / what can go wrong |
| --- | --- | --- |
| Effect and local decision can commit in the **same actual transaction** | Put the protected update and result/deduplication record in that transaction. | Only covers resources participating in that transaction. An HTTP call inside a transaction is not thereby included. |
| Recipient accepts a stable idempotency key | Repeat or query using the **same key and same intent**. | Verify key scope, retention, conflicting payload behaviour, and concurrent-attempt semantics. |
| Recipient supports immutable create-by-ID or a conditional transition | Retry the same identity/precondition, then inspect the established result. | A condition must protect the relevant effect. Do not accidentally overwrite a later legitimate state. |
| Recipient offers authoritative operation-status lookup | Query by operation identity; adopt confirmed outcome. | “Missing” must not coexist with a still-running request that can commit later. Otherwise remain Unknown. |
| Result is visible in an eventually consistent feed or query | Reconcile with a documented observation horizon and duplicate protection. | An empty read is weak negative evidence. Waiting longer without a contract is still not proof. |
| Recipient supports reservation and explicit confirmation | Recover the reservation and its state; confirm or cancel under its rules. | Expiry races and partial confirmations still need handling. |
| Effect is repeatable **in its actual context** | Retry according to that proven property. | Setting a value is not harmless if a stale retry overwrites a newer intended value; use version conditions. |
| A duplicate can be detected and compensated | Record the duplicate and run an identified compensation process. | This accepts a temporary duplicate. It is unsuitable when duplicates are forbidden or irreversible. |
| No reliable key, lookup, conditional operation, or acceptable duplicate exists | Stop automatic retries, preserve evidence, expose Unknown, and escalate to authorized reconciliation. | The business may need a different provider or a changed promise. |
| Evidence confirms the original effect did not happen and cannot still happen | A retry or new operation may be allowed by policy. | Distinguish “known absent” from “not yet observed.” |

This procedure does not guarantee that every external system is recoverable. It makes the limits reviewable.

Stripe is a concrete example of why an idempotency key is a contract, not a magic header. Its API v1 documentation specifies retained first responses, parameter comparison, and possible pruning after at least 24 hours. Reusing a pruned key can create a new request. Those details must shape the application's retry horizon. Do not generalize this retention window or error behaviour to every provider or to other API versions. [Stripe][s33].

### 6.5 Operation IDs, attempts, and business keys

Use one stable identity for a logical charge, shipment, document publication, or device action. Generate and retain it before the first uncertain attempt. An attempt number is for diagnostics and policy; it normally must not replace the deduplication key.

A conversation ID groups related work. It might contain separate operations: authorize, capture, and refund. Each effect can need its own identity. A business key such as “invoice + charge sequence” can be more durable across workflow restarts than an execution-local ID.

Do not deduplicate every equal amount paid to an account. Two identical-looking requests can be legitimate separate intentions. Conversely, if a retry changes a price lookup or target because the world changed, it may no longer be the same intention. Retain the decision inputs or reject the inconsistency.

An idempotent receiver needs to protect both the effect and the memory that identifies it. A caller-side “already sent” cache alone cannot establish receiver-side exactly-once effect semantics. [EIP Idempotent Receiver][s40].

### 6.6 Outbox, inbox, and reconciliation have separate jobs

An outbox records outgoing intent alongside a local state transition. A relay can retry delivery. This prevents the local “committed but forgot to notify” gap under the stated transaction and eventual-delivery assumptions. The relay can still send more than once. [Richardson][s32].

An inbox or equivalent retained operation record can atomically deduplicate a receiver's **local** state change. If that receiver then invokes a third-party service, a new effect gap exists. Naming the component an inbox does not remove it.

Reconciliation compares durable intention with independent evidence. It should have an explicit trigger, cadence, status model, and escalation threshold. It can be a per-operation process, a partitioned scan of unresolved records, or scheduled work on the owner. It need not be a singleton actor.

Reconciliation can discover that work succeeded late, failed, was partially applied, was duplicated, or was performed independently. Its output should become a normal domain message with evidence, not a direct undocumented rewrite of private state.

Compensation is a new business action. A refund is not deletion of a charge. Cancelling a shipment may fail after handover. Restoring old data can overwrite legitimate intervening work. Record compensation identity, progress, and failure as carefully as forward execution. [Compensating Transaction][s35].

### 6.7 Resource actors and digital twins

A device owner should distinguish:

- **Desired state:** what the system wants.
- **Reported state:** what the device last reported.
- **Observation metadata:** source, time, sequence, and device incarnation.
- **Pending commands:** what was requested and how completion will be established.
- **Confidence/availability:** connected, suspect, offline, or degraded.
- **Business lifecycle:** registered, enabled, retiring, retired.

A connection actor may be short-lived while the device identity persists. Reconnection must not make an old response applicable to a new command by accident. Include incarnation or generation where IDs can be reused.

A command acknowledgement may mean received, accepted, or physically achieved. For a valve, “command received” does not prove that it closed. For a printer, “job accepted” does not prove a page emerged. Model completion observations separately.

The Akka IoT example usefully separates networking from registered device actors. It does not implement a complete physical device protocol. The tutorial's temperature state must not be mistaken for a general guarantee that software controls device reality. [IoT registration][s17].

For an external account, a resource actor can own **our** sequence of intended operations and quota. It may not own the account's complete state if other applications or people can act on it. Reconciliation remains necessary.

### 6.8 OS processes and child programs

Erlang ports give a concrete resource-ownership idiom. A connected owner process exchanges data with an external OS program. The documentation explicitly qualifies external-program termination: the program must be written to terminate when the port closes. Actor death is therefore not a universal process-tree cleanup guarantee. [Erlang ports][s27].

A technology-agnostic child-program owner should specify:

- How a job is identified before launch.
- Starting, ready, busy, stopping, exited, and unknown states.
- How requests and output are correlated.
- Whether exit status establishes business completion or merely program termination.
- How to distinguish old output from a restarted program's output.
- Who handles stderr, malformed output, missing readiness, and stalled progress.
- Graceful shutdown followed by bounded escalation where supported.
- Whether grandchildren can outlive the owned process.
- What files, network effects, and external tasks remain after termination.
- Whether recovery reconnects to an existing job or starts a new one.

These are **synthesis requirements** from explicit resource ownership. They are not claims about what Erlang ports implement automatically.

For a transcoder, write to a unique job artifact, verify it, then publish a stable reference. For an external trading command, process exit is not evidence that no order reached the exchange. Both are “child processes,” but their effect contracts differ.

### 6.9 Inbound webhooks, polling, and change feeds

At ingress, authenticate the source and translate external identity into a tenant-scoped domain target. Preserve an event ID, relevant source version, and evidence required for investigation. Distinguish accepting an input durably from completing its business reaction.

Stripe's webhook guidance documents duplicate events and lack of guaranteed event ordering. It recommends signature verification and asynchronous handling. These are concrete reasons to avoid assuming a webhook is a unique, ordered command. [Stripe webhooks][s74].

**Synthesis for other feeds:**

- For duplicate delivery of one event, deduplicate its event identity.
- For two different events describing the same business transition, use domain transition identity as well.
- For out-of-order observations, use source versions if available or re-read authoritative state.
- For a polling cursor, define whether advancing it and retaining discovered work are atomic. If not, prefer safe replay over silent gaps.
- For change feeds, define snapshot/cursor handover and recovery from cursor expiry.
- For a webhook gap, decide whether polling can repair it or an operator must investigate.
- For callbacks that arrive before registration or after closure, define retention, rejection, or reconciliation.

An event may be a snapshot, a delta, or a hint to fetch current state. These require different protocols. Do not treat every notification as a complete and authoritative historical record.

### 6.10 Quotas, rate limits, slow calls, and circuit breakers

A quota actor should match the scope of the real allowance: merchant account, API credential, device, tenant, or global contract. Decide whether admission consumes a token immediately, reserves it, or refunds it after rejection. Define fairness and maximum pending work.

A global allowance cannot safely be enforced by unrelated local counters without an allocation or coordination rule. If exact global enforcement is unnecessary, document the permitted overshoot. If exactness is required, central authority or allocated rights may be appropriate.

A circuit breaker is dependency-health state. Closed permits normal calls; Open rejects or defers; Half-open admits controlled probes. A business refusal such as “insufficient funds” should not automatically count as dependency failure. Separate breaker scope from quota scope. [Circuit Breaker][s31].

**If handlers exclude subsequent requests while awaiting a slow call**, move long work into an explicit pending conversation, delegate execution, or choose a different abstraction. **If other requests can interleave**, ensure the owner's invariants remain true across the await. [Orleans scheduling][s21].

The boundary contract should specify whether outages yield immediate refusal, stale data, durable deferred work, partial results, or escalation. Silent accumulation is not a policy. Halo's launch incident illustrates the need to expect harmful client traffic even from a first-party application. [McCaffrey's incident report][s57].

### 6.11 Files, hardware, people, clocks, and randomness

These are synthesis examples of applying the same effect analysis:

- **Files:** a pathname is not an immutable content identity. Define partial-write visibility, version checks, and publication. If using atomic replacement, verify the relevant filesystem contract. Concurrent writers still require authority.
- **Hardware:** model desired versus measured state and command generations. A retry must be safe for the physical operation; “dispense one item” is unlike “set target temperature.”
- **Humans:** retain task and proposal version. Validate authority when a decision arrives. A second click, email, or delayed callback must not approve a different revision. [Azure human interaction][s70].
- **Clocks:** distinguish business timestamps from elapsed-time measurement. State whose time decides expiry and what skew or delayed notification can change. A timeout schedules reconsideration; it is not an observation that a remote action stopped.
- **Randomness:** generate an effect ID once and retain it. In replayed workflows, use supported deterministic or recorded time/randomness facilities. Do not repeat a lottery draw after recovery unless that is the intended rule. Restate documents recorded nondeterminism and deterministic helpers, including that its deterministic random helpers are not for cryptographic use. [Restate durable steps][s75].
- **Email and notifications:** distinguish provider acceptance from user receipt or reading. If the provider cannot deduplicate and duplicates matter, preserve uncertainty rather than pretending the actor can prove one delivery.

### 6.12 What each actor should promise to its users

A reviewable contract can read:

> Commands for this booking are validated against its current version. An Accepted reply means the operation and its required follow-up have been retained under the stated durability assumption. Repeating the same operation ID with the same intent returns its current or established result during the retention period. Completion means the named external evidence has been recorded. A timeout does not imply cancellation. Unknown operations remain queryable and owned by reconciliation until resolved or explicitly escalated.

This is a **template**, not an automatic actor guarantee. Replace every promise with the actual chosen semantics. If Accepted only means “queued in volatile memory,” say that instead.

## 7. Review checklist and design-level verification

Answer **Yes**, **No**, or **Not applicable with reason**. For each Yes, name the design artifact or code location that supplies evidence. These checks are synthesis tied to the principles and anti-patterns above.

### Boundaries and ownership

- [ ] Does every actor have a concise decision, activity, or resource responsibility? P01; AP01/AP06.
- [ ] Does every immediate invariant have one authority or a precise coordination mechanism? P02; AP09.
- [ ] Are actor IDs canonical, tenant-scoped where needed, and stable across retries? P09/P15.
- [ ] Are key reuse, deletion, and delayed messages addressed? P11/P15.
- [ ] Are business lifecycle and running-instance lifetime distinguished? A2/A3; AP16.
- [ ] Are creation, dependency, and supervision relationships separately justified? P07/A5.
- [ ] Are functions and values used where independent actors add no benefit? P13; AP14.
- [ ] Is every singleton's scope justified by a real singular authority? AP08.

### State

- [ ] Can only the declared owner mutate authoritative state? P04; AP07.
- [ ] Are messages and replies immutable or transferred under enforceable ownership rules? AP10.
- [ ] Are observed and projected data distinguished from authoritative state? P05; AP22.
- [ ] Are legal states and invalid message/state combinations specified? B1/C4.
- [ ] Are stale reads prevented from authorizing unchecked writes? P02; AP02.
- [ ] Are state, history, subscriber, and pending-work bounds documented? P12; AP13.
- [ ] Does retention preserve the declared retry and deletion contracts? B5/C5.
- [ ] Do read models state freshness and rebuild behaviour? B4.

### Protocols

- [ ] Does each public message express a clear command, event, query, or reply? P03/C1.
- [ ] Does every acknowledgement state what it proves? P08.
- [ ] Are logical operation, conversation, and attempt identities unambiguous? P09/C2.
- [ ] Are duplicate and conflicting-key behaviours defined? C5.
- [ ] Are response correlation and reply destination explicit or guaranteed by a documented abstraction? C2.
- [ ] Are unknown and late replies handled deliberately? P05/P11.
- [ ] Are internal messages separated from public contracts? C3.
- [ ] Is every necessary protocol version supported or migrated explicitly? P15/C8.
- [ ] Is each ask justified by a real need for its result? AP04/AP05.
- [ ] Does the protocol avoid hidden ordering assumptions across senders? P14; AP12.

### Coordination and time

- [ ] Is the waiting graph free of unanalysed cycles? AP11.
- [ ] Are invariants across awaits valid under the actual interleaving assumptions? P02/P14.
- [ ] Is orchestration or choreography chosen for explicit reasons? D3/D4.
- [ ] Does someone own end-to-end completion and exception resolution? P05/D3/D4.
- [ ] Are compensation steps independently identified, repeatable, and recoverable? D2.
- [ ] Are reservation expiry and confirm/cancel races defined? D5.
- [ ] Are fan-in membership and completeness conditions fixed for each conversation? D7/D8.
- [ ] Are outstanding work and fan-out bounded? P12/D12.
- [ ] Does each deadline have an owner, clock assumption, and late-result policy? P11/E1.
- [ ] Are obsolete timers and reminders recognized? E1/E2.
- [ ] Can human tasks expire, be withdrawn, and reject obsolete approvals? E5.
- [ ] Is a missed heartbeat treated as suspicion unless stronger evidence exists? E4.

### Failure and external effects

- [ ] Are business refusal, transient fault, and unknown outcome distinct? P06; AP15/AP19.
- [ ] Is the state restored by restart explicitly described? P07; AP16.
- [ ] Are risky work and valuable state isolated where their failure needs differ? F2.
- [ ] Is escalation bounded and assigned to an accountable owner? F3.
- [ ] Is one retry policy responsible for each logical operation? F4.
- [ ] Is poison work parked or failed deliberately with repair ownership? AP20.
- [ ] Does each external effect have a retained identity and immutable intent? P09/G2.
- [ ] Has the post-effect/pre-recording crash gap been analysed? P10; AP17.
- [ ] Does the chosen branch of section 6.4 match the recipient's actual guarantees? G2.
- [ ] Are outbox guarantees kept separate from effect guarantees? AP18.
- [ ] Can late success be reconciled after timeout or cancellation? AP19.
- [ ] Are external resource and child-program shutdown limits explicit? A3/G1.
- [ ] Are webhook duplicates, ordering, and missed events addressed? P14/G2.
- [ ] Do quota and circuit scopes match the real dependency? G4.
- [ ] Do stable disconnected/degraded states exist where appropriate? AP21.
- [ ] Are guarantees to callers stated in domain language? P03/P08.

### 7.1 Verify properties, not just example responses

The verification plan below is synthesis. State the environment assumptions before claiming liveness.

**Safety examples:**

- No seat is confirmed for two bookings.
- An operation key never binds to two different intentions.
- A confirmed outcome cannot revert to a weaker observation through an old reply.
- Cancellation cannot make a late confirmed effect disappear from the record.
- Allocated rights never exceed the original budget.
- A stale generation cannot authorize a new physical or business action.

**Liveness examples:**

- Under eventual delivery and a recovering dependency, every accepted operation eventually reaches a terminal state or an explicit owned escalation.
- An expired reservation eventually becomes reusable unless confirmation already won.
- Every parked operation remains discoverable by its assigned recovery process.
- Every pending human task eventually receives a decision, expiry, or escalation under the scheduler assumptions.

“Escalated” is a completed protocol response, not necessarily completed business work. Do not use it to claim the real-world obligation vanished.

Use a small model of owners, state, messages, and external evidence. Generate duplicates, omissions permitted by the delivery model, reorderings, time advances, crash/restart points, old generations, invalid input, and conflicting retries. Crash particularly before and after each durability or external-effect boundary.

Check state-machine properties after every transition. Compare implementation traces to a simpler model. Use virtual time for deadline tests. Test the same logical operation with both identical and conflicting payloads. A deterministic replay of one known trace does not explore all traces.

### 7.2 Verification approaches and their limits

| Approach | Best design use | Limit |
| --- | --- | --- |
| State tables and sequence traces | Expose missing transitions, obligations, and cycles cheaply. | Humans miss combinations. |
| Model-based property tests | Generate message histories and compare invariants with an executable reference model. | Sampling is not exhaustive; the generator can omit the actual fault. |
| TLA+ | Specify state transitions, safety, and liveness; explore small but adversarial configurations. | The model's assumptions and abstraction must match the intended implementation. |
| P | Model asynchronous participants as communicating state machines; check unhandled events and progress conditions. | Requires modelling the environment and choosing suitable exploration bounds. |
| Rebeca / Timed Rebeca | Actor-oriented state models, timing constraints, and requirement-to-model traceability. | Finite abstractions and timing assumptions constrain conclusions. |
| Session types / monitors | Describe permitted conversation roles, choices, and message sequences. | Payload correctness, crashes, external effects, and business invariants need additional reasoning. |

The AWS TLA+ experience report supports design-level model checking as a complement to code testing. Desai et al. show P's event-state modelling, including safety and liveness checks. Sirjani et al. demonstrate requirement-driven Rebeca models on passenger-door control. Fowler demonstrates session monitoring in Erlang with chat and DNS examples. These are different tools, not competing proofs that actors are automatically correct. [TLA+ experience][s55]; [P][s52]; [Rebeca][s53]; [session actors][s51].

The empirical critique is concrete. Tasharofi et al. studied 15 Scala actor programs; 80% mixed concurrency models. This is a result about that sample and period, not today's universal rate. Bagherzadeh et al. analysed 186 Akka bugs and found several root-cause categories; typing alone was not the whole problem. Torres Lopez et al. distinguish progress failures and protocol violations. Their findings support inspecting both communication and any escape from actor isolation. [Scala study][s48]; [Akka study][s49]; [taxonomy][s50].

### 7.3 Splitting and merging a design

Use semantic evidence before structural refactoring.

**Split candidates:** unrelated invariants, distinct business lifetimes, independent external resources, separate recovery needs, or one conversation's pending state obscuring others.

**Merge candidates:** most work requires reciprocal calls; state changes must always be agreed together; a child has no separate authority or lifecycle; inter-actor communication exists only to mimic local field access.

A safe conceptual migration plan is synthesis:

1. Restate the old observable contract and invariants.
2. Name the authority before, during, and after migration.
3. Preserve or explicitly translate identities and deduplication evidence.
4. Route unfinished conversations and old replies intentionally.
5. Transfer state and outstanding obligations under a mechanism that prevents conflicting writers.
6. Preserve tombstones or redirects for delayed work where required.
7. Verify old/new interaction traces before retiring the old protocol.

Moving code between actors is not enough. A split changes where a previously local decision becomes a protocol. A merge may change independence, cancellation, and failure scope. [Vernon I][s08]; [Vernon II][s09]; [Orleans interface versioning][s66].

### 7.4 Published examples and what to learn from them

- **Akka IoT:** inspect device ownership, registration, and a separate query conversation. The query distinguishes no reading, actor unavailable, and deadline expiry. The networking and physical-device protocol remain outside the tutorial. Use it as a protocol-design exercise, not a complete IoT reference architecture. [IoT part 4][s17]; [IoT part 5][s18].
- **Orleans samples:** chat demonstrates streams; the bank-account sample demonstrates opt-in transactions. Compare their required assumptions before extracting general rules. [Samples][s56]; [transactions][s61].
- **Halo:** the 2014 Orleans paper uses player/game presence as a stateful middle-tier example. It supports domain-keyed ownership, not the claim that any granularity will work. [Bernstein et al.][s13]. McCaffrey's launch report adds an important boundary lesson: first-party clients can produce pathological traffic. [Incident][s57].
- **Erlang/OTP:** Armstrong's telecom case studies ground responsibility and fault isolation. Use them for recovery reasoning rather than importing telecom availability figures into a different system. [Thesis][s03].
- **WhatsApp:** Rick Reed's 2014 slides give a firsthand architecture and operational account. Most detail concerns scale and operations, outside this report's scope. They are evidence of practice, not sufficient evidence for a precise universal “one actor per X” decomposition rule. [Slides][s58].
- **Discord:** its session and guild processes give concrete lifecycle and domain boundaries. The Maxjourney report describes a successful guild owner that later needed fan-out responsibilities moved into a sender process. This is evidence for evolving responsibilities without assuming the original entity boundary was wholly wrong. [Discord][s59].
- **Rebeca passenger-door control:** requirements become state models and checked properties. This is a stronger model for a design-review method than learning only from production success stories. [Sirjani et al.][s53].

## 8. Where framework and design advice differs

These are differences in abstraction, assumptions, or emphasis. They are not always contradictions.

| Design issue | Advice or model A | Advice or model B | How to decide |
| --- | --- | --- | --- |
| What deserves an actor? | Armstrong: independently concurrent activities and resources. [S03][s03]; [S62][s62] | Orleans/Dapr: many relatively independent keyed entities; Cloudflare: logical coordination units. [S14][s14]; [S22][s22]; [S24][s24] | Start with domain responsibility; distinguish activity, entity, and resource lifetimes. |
| Everything as actors? | The foundational formalism can represent functions and data structures as actors. [S01][s01] | Production guidance recommends stateless functions for work without coordination needs. [S24][s24] | A universal computational model does not require an application-level actor object for every value. |
| Creation and termination | OTP/Akka/Proto.Actor expose explicit responsibility and supervision relationships. [S04][s04]; [S15][s15]; [S25][s25] | Orleans virtual identities outlive running instances; Dapr uses a related abstraction. [S13][s13]; [S22][s22] | Keep business creation/deletion in the model even when instance lifecycle is supplied. |
| Request/reply | Message-first idioms emphasize explicit replies and forward flow. [S16][s16] | Orleans deliberately supports asynchronous object-like calls. [S14][s14] | Both can be idiomatic. Review failure, latency, and waiting dependencies rather than syntax. |
| Await and isolation | Default non-reentrant Orleans requests run to completion relative to other requests. [S21][s21] | Reentrant modes and Cloudflare external-I/O awaits can allow interleaving. [S21][s21]; [S24][s24] | Write the condition that actually holds; preserve invariants across suspension points. |
| Cross-actor consistency | Helland assumes no distributed transaction; Vernon advocates small aggregate boundaries and eventual coordination. [S10][s10]; [S09][s09] | Current Orleans offers opt-in distributed ACID transactions; Azure entities offer durable critical sections. [S61][s61]; [S26][s26] | Use guarantees appropriate to the chosen system. Azure locks are not automatic rollback; external effects remain separate. |
| Recovery emphasis | OTP separates fault recovery from normal work and restores stable services. [S04][s04]; [S06][s06] | Virtual actors hide instance recreation; durable workflows preserve process progress. [S13][s13]; [S28][s28] | Hidden recovery machinery does not remove business recovery or external reconciliation. |
| Long-running activity | Dapr/Orleans suitability advice cautions against lengthy blocking work. [S22][s22]; [S14][s14] | Temporal and Azure workflows explicitly model long waits and approvals. [S29][s29]; [S70][s70] | Distinguish durable logical waiting from monopolizing an entity's request handling. |
| How small? | Small independent actors improve fault boundaries. [S63A][s63a] | Tiny chatty grains are discouraged. [S14][s14] | Split independent responsibility, not each method; merge when nearly every decision must cross the boundary. |
| Retry | A restart may abandon the failed request; application delivery rules matter. [S25][s25] | Durable execution can retry unfinished work according to policy. [S28][s28]; [S30][s30] | Define operation identity and effect safety regardless of retry location. |
| Transactions over time | Original sagas decompose long database transactions with compensations. [S11][s11] | Modern distributed sagas span services and trust boundaries. [S12][s12] | Do not import the original paper's database recovery assumptions into arbitrary external services. |
| Queries | Asking an actor gives one owner's response. [S18][s18] | CQRS supplies separate query models; Fowler warns of added complexity. [S34][s34] | Choose by freshness, authority, and query shape. CQRS is optional. |
| Exactly once | Durable systems can deduplicate internal invocations or reuse journaled results. [S30][s30] | External work may execute again before completion is recorded. [S28][s28] | State exactly which boundary and effect the guarantee covers. |

Historical caution: the 2011 Orleans paper discusses a prototype with transaction ideas. The 2014 paper describes a different design and explicitly lists limitations of that version. Current transaction documentation must be used for current capability claims. Do not combine these texts into a fictional timeless Orleans contract. [Bykov et al.][s60]; [Bernstein et al.][s13]; [current transactions][s61].
## 9. Annotated bibliography and access record

This is a selective reading record, not a claim that every long book or manual was read cover to cover. “Accessible” means the research tool returned the text or relevant passages. Framework pages are living documents, consulted for this report on 15 November 2026. Verify their current version when applying a framework-specific guarantee. Public EIP pages are excerpts, not the entire book.

### 9.1 Sources used or explicitly assessed

- **[S01 — Hewitt, Bishop, and Steiger (1973), A Universal Modular ACTOR Formalism for Artificial Intelligence][s01].** Foundational paper. Full PDF accessible; selected passages read. Supports the message-oriented formalism and explicit obligations, not every modern operational recommendation.
- **[S02 — Agha (1986), Actors: A Model of Concurrent Computation in Distributed Systems][s02].** Publisher record only; the book's full text was not reached. Listed for provenance, not used as if its chapters had been read. See access limitations below.
- **[S03 — Armstrong (2003), Making reliable distributed systems in the presence of software errors][s03].** Primary thesis. Full PDF accessible; design, concurrency, supervision, and error-kernel passages read. The strongest source here for modelling independent activity and recovery responsibility.
- **[S04 — Erlang/OTP, Supervisor Behaviour][s04].** Official design principles. Read recovery strategies and escalation. A supervision tree expresses recovery coupling; this does not automatically make it the domain ownership tree.
- **[S05 — Erlang/OTP, gen_statem Behaviour][s05].** Official state-machine guidance. Selected event, transition, and postponement sections read. Useful for explicit protocol states and state-dependent handling.
- **[S06 — Fred Hebert, It's About the Guarantees][s06].** Practitioner essay read. Explains recovery as restoration of known guarantees; a stable disconnected state can be valid.
- **[S07 — Fred Hebert (2016), Erlang in Anger, version 1.1.0][s07].** Author-hosted PDF accessible; selected design and production-safety sections read. Operational details were excluded except where they reveal responsibility or bounds.
- **[S08 — Vaughn Vernon (2011), Effective Aggregate Design, Part I: Modeling a Single Aggregate][s08].** Primary practitioner paper read. Invariant-led boundaries and problems with oversized aggregates; an aggregate need not be an actor.
- **[S09 — Vaughn Vernon (2011), Effective Aggregate Design, Part II: Making Aggregates Work Together][s09].** Primary practitioner paper read. Identity references and coordination across aggregate boundaries. Used with Part I, not as a universal ban on transactions.
- **[S10 — Pat Helland (2007), Life Beyond Distributed Transactions: an Apostate's Opinion][s10].** Primary paper; entity and activity passages read. Treats local partner knowledge and obligations under an explicit no-distributed-transaction assumption. Its activity is not a Temporal Activity.
- **[S11 — Hector Garcia-Molina and Kenneth Salem (1987), Sagas][s11].** Primary paper available in an academic course archive; selected text read. Database sagas and compensation. Modern cross-service use requires additional failure and isolation reasoning.
- **[S12 — Caitie McCaffrey (2017), Distributed Sagas: A Protocol for Coordinating Microservices][s12].** Firsthand talk/slides, with accessible transcript material. Useful modern application of sagas; does not establish arbitrary external rollback.
- **[S13 — Bernstein, Bykov, Geller, Kliot, and Thelin (2014), Orleans: Distributed Virtual Actors for Programmability and Scalability][s13].** Primary technical report. PDF and selected programming-model/Halo passages consulted. Describes a historical system; use current docs for current feature claims.
- **[S14 — Microsoft Orleans, Best practices][s14].** Official guidance read. Entity suitability, asynchronous collaboration, and avoiding chatty or concentrated designs. These are Orleans recommendations, not actor axioms.
- **[S15 — Akka, How the Actor Model Meets the Needs of Modern, Distributed Systems][s15].** Official introduction read for failure semantics and actor relationships. Supports distinguishing domain outcomes from component faults.
- **[S16 — Akka Typed, Interaction Patterns][s16].** Official catalogue; selected request/reply, adapters, session, and timeout patterns read. An asynchronous ask is a supported idiom, not automatically an anti-pattern.
- **[S17 — Akka IoT tutorial, Part 4: Working with Device Groups][s17].** Worked example read. Device identities, registration, and group responsibility. The tutorial explicitly abstracts away the physical networking layer.
- **[S18 — Akka IoT tutorial, Part 5: Querying Device Groups][s18].** Worked example read. A distinct conversation owns collection membership, deadline, and several partial-result outcomes.
- **[S19 — Apache Pekko, Actors][s19].** Official typed-actor introduction and chat-room example consulted. Useful for explicit session participants and state-dependent behaviour.
- **[S20 — Apache Pekko, Style Guide][s20].** Official style guidance consulted alongside the corresponding Akka page. Public versus internal protocol organization is design guidance; typing does not prove conversation correctness.
- **[S21 — Microsoft Orleans, Request scheduling][s21].** Official documentation read. Concrete cyclic-call deadlock and reentrancy examples; supports conditional reasoning about awaits.
- **[S22 — Dapr, Actors overview][s22].** Official suitability and model guidance read. Relatively independent entities fit well; long-running blocking work needs a different design.
- **[S23 — Dapr, Actors timers and reminders][s23].** Official documentation read. Timers and persisted reminders have different survival contracts. The report avoids importing version-specific retry defaults.
- **[S24 — Cloudflare, Rules of Durable Objects][s24].** Official design guidance read. Scope an object to a coordination unit; external-I/O awaits and volatile memory require explicit assumptions.
- **[S25 — Proto.Actor / Asynkron, Supervision][s25].** Official supervision documentation read. Recovery actions and the distinction between restarting a component and retrying failed business work.
- **[S26 — Microsoft, Durable Task entities][s26].** Official overview and critical-section guidance read. Durable entities and orchestrations offer different responsibilities; entity locks do not imply automatic transactional rollback.
- **[S27 — Erlang/OTP, Ports][s27].** Official documentation read. External-program ownership and communication; termination of an external program depends on it handling the closed port appropriately.
- **[S28 — Temporal, Activity Definition][s28].** Official documentation, especially idempotency, read. An unfinished activity can execute again after its effect happened but before completion became durable.
- **[S29 — Temporal, Workflow message passing][s29].** Official documentation read selectively for queries, signals, updates, and waiting. Language-specific APIs illustrate framework-independent conversation distinctions.
- **[S30 — Restate, Why we built Restate][s30].** First-party design explanation read, including the caveat about external side effects executing again before their result is recorded. Marketing shorthand must be read with that boundary.
- **[S31 — Microsoft Azure Architecture Center, Circuit Breaker pattern][s31].** Official pattern guidance read. Failure admission and recovery probes are separate from retry policies.
- **[S32 — Chris Richardson, Transactional outbox][s32].** Author-maintained pattern page read. Local state and outgoing intent commit together; publication may repeat.
- **[S33 — Stripe, Idempotent requests][s33].** First-party API contract read. A concrete example of recipient deduplication, payload checks, and retention limitations. API versions and operations can differ.
- **[S34 — Martin Fowler, CQRS][s34].** Practitioner explanation read. Separating read and write models is optional and can add substantial complexity.
- **[S35 — Microsoft Azure Architecture Center, Compensating Transaction pattern][s35].** Official pattern guidance read. Compensation is application-specific, can fail, and need not restore the exact original world.
- **[S36 — Hohpe and Woolf, Aggregator][s36].** Public pattern excerpt read. Correlation, completeness, and combination decisions.
- **[S37 — Hohpe and Woolf, Process Manager][s37].** Public pattern excerpt read. Explicit process progress and branching; centralized coordination can be unnecessary.
- **[S38 — Enterprise Integration Patterns, Try-Confirm/Cancel][s38].** Public conversation-pattern page read. Tentative commitment and expiry; a useful extension of the messaging catalogue.
- **[S39 — Hohpe and Woolf, Claim Check][s39].** Public pattern excerpt read. External payload references; the report adds lifecycle and immutable-version review questions as synthesis.
- **[S40 — Hohpe and Woolf, Idempotent Receiver][s40].** Public pattern excerpt read. Duplicate suppression or operations whose semantics tolerate repetition.
- **[S41 — Hohpe and Woolf, Correlation Identifier][s41].** Public pattern excerpt read. Connects replies to requests; business-operation identity may be a separate key.
- **[S42 — Hohpe and Woolf, Pipes and Filters][s42].** Public pattern excerpt read. Forward composition of stages; independent stage responsibility must justify actor boundaries.
- **[S43 — Hohpe and Woolf, Publish-Subscribe Channel][s43].** Public pattern excerpt read. Independent consumers of a fact; does not by itself establish completion of a business process.
- **[S44 — Hohpe and Woolf, Routing Slip][s44].** Public pattern excerpt read. Carrying a route with work; mutation and trust of the route need a contract.
- **[S45 — Hohpe and Woolf, Competing Consumers][s45].** Public pattern excerpt read. Distributing independent work; completion and repeated execution still need ownership.
- **[S46 — Hohpe and Woolf, Dead Letter Channel][s46].** Public pattern excerpt read. Separating undeliverable work. An application parking lot needs more business context than a runtime dead-letter log.
- **[S47 — Hohpe and Woolf, Messaging Gateway][s47].** Public pattern excerpt read. Encapsulating communication; the report's anti-corruption and effect-ledger designs are extensions, not quotations of this short page.
- **[S48 — Tasharofi, Dinges, and Johnson (2013), Why Do Scala Developers Mix the Actor Model with Other Concurrency Models?][s48].** ECOOP paper accessible; abstract and selected study material consulted. Fifteen-program study. Its prevalence result is historical and sample-specific.
- **[S49 — Bagherzadeh, Fireman, Shawesh, and Khatchadourian (2020), Actor concurrency bugs: a comprehensive study on symptoms, root causes, API usages, and differences][s49].** OOPSLA paper accessible; abstract and selected categories consulted. Study of 186 Akka bugs; useful counterevidence to claims that isolation or typed messages remove protocol errors.
- **[S50 — Torres Lopez, Marr, Gonzalez Boix, and Mössenböck, A study of concurrency bugs and advanced development support for actor-based programs][s50].** Full public preprint accessible; taxonomy sections read. Progress failures and behavioural protocol failures support different review questions. Preprint and publication dates differ.
- **[S51 — Simon Fowler (2016), An Erlang Implementation of Multiparty Session Actors][s51].** Research paper accessible; abstract and protocol-monitoring discussion read. Chat and DNS examples; session conformance is narrower than complete business correctness.
- **[S52 — Desai et al. (2013), P: Safe Asynchronous Event-Driven Programming][s52].** Primary publication page and abstract read. Event-driven state machines and verification. No claim here that merely adopting P proves an arbitrary implementation.
- **[S53 — Sirjani et al. (2019), From Requirements to Verifiable Executable Models Using Rebeca][s53].** Primary paper accessible; abstract and requirements-to-model material read. Passenger-door example connects modelling with properties.
- **[S54 — Alberto Brandolini / EventStorming, official overview][s54].** Method overview read. Supports collaborative discovery of events and policies; the actor-design workflow in this report is original synthesis.
- **[S55 — Newcombe et al., Use of Formal Methods at Amazon Web Services][s55].** Primary experience report accessible; opening and selected design-verification material read. Supports lightweight model checking before implementation; not specific to actor runtimes.
- **[S56 — Microsoft Orleans, Tutorials and samples][s56].** Official sample descriptions consulted. Chat and bank-account examples illustrate different assumptions; their code was not independently audited.
- **[S57 — Caitie McCaffrey (2015), Clients Are Jerks: How Halo 4 DoSed the Services at Launch and How We Survived][s57].** Firsthand experience report read. Client behaviour can invalidate assumed traffic patterns; supports explicit boundary admission and retry ownership.
- **[S58 — Rick Reed (2014), Scaling to Millions of Simultaneous Connections, WhatsApp][s58].** Firsthand presentation accessible; architecture and context slides consulted. Most operational content was intentionally excluded.
- **[S59 — Discord (2023), Maxjourney: Pushing Discord's Limits with a Million+ Online Users in a Single Server][s59].** First-party experience report read for session/guild ownership and moving delivery work to a sender process. Runtime tuning details excluded.
- **[S60 — Bykov et al. (2011), Orleans: Cloud Computing for Everyone][s60].** Primary paper accessible; abstract and model discussion consulted. Historical prototype, including computational workloads and transaction ideas; not current Orleans documentation.
- **[S61 — Microsoft Orleans, Transactions][s61].** Current official documentation read. Opt-in ACID transactions qualify the blanket claim that actors can never coordinate transactionally.
- **[S62 — Joe Armstrong (2003), Concurrency Oriented Programming in Erlang][s62].** Primary paper accessible; modelling discussion consulted. Independent concurrent activities as a decomposition starting point.
- **[S63 — Hohpe and Woolf, Command Message][s63].** Public pattern excerpt read. A request for work differs semantically from a notification that something happened.
- **[S63A — Fred Hebert (2016), The Zen of Erlang][s63a].** Practitioner essay read for isolation, relationships, and recovery. A design perspective, not a claim that arbitrary crashing is harmless.
- **[S64 — Hohpe and Woolf, Event Message][s64].** Public pattern excerpt read. Facts can inform multiple consumers without commanding a specific response.
- **[S65 — Hohpe and Woolf, Return Address][s65].** Public pattern excerpt read. The reply destination is a protocol concern, even when a framework hides its representation.
- **[S66 — Microsoft Orleans, Grain versioning][s66].** Official interface-versioning documentation read. Interface compatibility is distinct from state and in-flight business-process migration.
- **[S67 — Chris Richardson, Saga][s67].** Author-maintained pattern page read. Orchestration/choreography comparison and explicit lack of saga isolation.
- **[S68 — Balegas et al. (2015), Extending Eventually Consistent Cloud Databases for Enforcing Numeric Invariants][s68].** Primary paper accessible; abstract and introduction read. Allocated rights can preserve certain numeric bounds. It is not a general method for arbitrary global invariants.
- **[S69 — Hohpe and Woolf, Scatter-Gather][s69].** Public pattern page consulted. Multiple recipients and collection of replies; completeness remains a separate design decision.
- **[S70 — Microsoft, Human interaction in Durable Task][s70].** Official overview read. Durable approval waits and deadlines; the report adds stale-approval and authority questions as synthesis.
- **[S71 — Temporal, Design Patterns][s71].** Official catalogue consulted for retry, cancellation, and workflow considerations. Exact APIs and defaults are outside this report.
- **[S72 — Proto.Actor, Actors][s72].** Official actor guidance consulted. Delivery and failure assumptions must be separated from application acknowledgements.
- **[S73 — Pat Helland and Dave Campbell (2009), Building on Quicksand][s73].** Primary paper accessible; abstract and opening model read. Its asynchronous durability/replication assumptions motivate caution; they are not a universal actor contract.
- **[S74 — Stripe, Receive Stripe events in your webhook endpoint][s74].** First-party documentation read, including duplicate events, non-guaranteed ordering, and source verification. Concrete inbound integration evidence.
- **[S75 — Restate, Durable steps][s75].** Official documentation read. Recording nondeterministic results and using replay-safe time/randomness helpers; deterministic randomness is not automatically suitable for secrets.

### 9.2 Books, unavailable sources, and partial access

- **Gul Agha, Actors (1986).** The [MIT Press record][s02] was reached. The [MIT repository record for the 1985 precursor thesis](https://dspace.mit.edu/entities/publication/0b4cb2ed-5ea9-4ca9-b98e-1f440c654f3d) exposed an abstract, but the [thesis PDF](https://dspace.mit.edu/bitstream/handle/1721.1/6952/AITR-844.pdf) returned a rate-limit error. The 1985 thesis and 1986 book are distinguished. No chapter-specific recommendation is attributed to unread text.
- **Pat Helland, Idempotence Is Not a Medical Condition (2012).** Located bibliographically. Both the [ACM Queue article](https://queue.acm.org/detail.cfm?id=2187821) and [CACM article](https://cacm.acm.org/practice/idempotence-is-not-a-medical-condition/) denied access to the research tool. The substantive idempotency guidance instead cites the accessible receiver pattern, provider contract, and durable-execution documentation. This article was not cited from memory.
- **Francesco Cesarini and Steve Vinoski, Designing for Scalability with Erlang/OTP.** The [publisher preview and contents](https://www.oreilly.com/library/view/designing-for-scalability/9781449361556/) were reachable; full chapters were not. Relevant further reading, not a substantive source for unverified chapter claims.
- **Vaughn Vernon, Reactive Messaging Patterns with the Actor Model.** The [publisher preview and contents](https://www.oreilly.com/library/view/reactive-messaging-patterns/9780133846904/) were reachable; the complete book was not. The accessible Effective Aggregate Design papers and public EIP catalogue supply the report's specific claims.
- **Roland Kuhn, Brian Hanafee, and Jamie Allen, Reactive Design Patterns.** The [publisher description and contents](https://www.manning.com/books/reactive-design-patterns) were reachable; full chapters were not. Listed as relevant further reading only.
- **Erlang in Anger.** An initial official-site route failed. The [author-hosted PDF][s07] was accessible and used, so the work itself was not inaccessible.
- **Armstrong's thesis.** One academic mirror failed. The [erlang.org PDF][s03] was accessible and used.
- **Proto.Actor supervision.** An older documentation path returned not found. The [current Asynkron documentation][s25] was accessible and used.

### 9.3 Evidence limits and next research priorities

The strongest evidence here is convergence among explicit framework contracts, foundational papers, messaging patterns, and worked examples. Decomposition remains a design judgement. No source establishes a universal optimal actor size, a rule that every aggregate must be an actor, or a rule that every conversation must avoid request/reply.

The empirical studies concern particular languages, projects, and historical periods. They identify credible failure modes; they do not estimate the defect rate of every actor system. Production reports are useful examples but have selection bias. Successful deployments do not prove that actors were necessary.

The original seat/payment example, effect-decision guide, responsibility cards, and checklist are **synthesis**. They are intended for critique and adaptation, not as results of a published controlled study. A future skill should keep its runtime assumptions explicit and ask for counterexample traces before recommending a substantial redesign.

Useful follow-up reading would be the unavailable book chapters, the full Helland idempotence essay, and implementation-specific verification of any selected platform's persistence, reminder, idempotency-retention, and external-call contracts. None is required to treat an unknown external outcome honestly.

[s01]: https://ijcai.org/Proceedings/73/Papers/027B.pdf
[s02]: https://mitpress.mit.edu/9780262511414/actors/
[s03]: https://erlang.org/download/armstrong_thesis_2003.pdf
[s04]: https://www.erlang.org/doc/system/sup_princ.html
[s05]: https://www.erlang.org/doc/system/statem.html
[s06]: https://ferd.ca/it-s-about-the-guarantees.html
[s07]: https://s3.us-east-2.amazonaws.com/ferd.erlang-in-anger/text.v1.1.0.pdf
[s08]: https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_1.pdf
[s09]: https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_2.pdf
[s10]: https://www.cidrdb.org/cidr2007/papers/cidr07p15.pdf
[s11]: https://www.cs.cornell.edu/andru/cs711/2002fa/reading/sagas.pdf
[s12]: https://speakerdeck.com/caitiem20/distributed-sagas-a-protocol-for-coordinating-microservices
[s13]: https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/Orleans-MSR-TR-2014-41.pdf
[s14]: https://learn.microsoft.com/en-us/dotnet/orleans/resources/best-practices
[s15]: https://doc.akka.io/libraries/akka-core/current/typed/guide/actors-intro.html
[s16]: https://doc.akka.io/libraries/akka-core/current/typed/interaction-patterns.html
[s17]: https://doc.akka.io/libraries/akka-core/current/typed/guide/tutorial_4.html
[s18]: https://doc.akka.io/libraries/akka-core/current/typed/guide/tutorial_5.html
[s19]: https://pekko.apache.org/docs/pekko/current/typed/actors.html
[s20]: https://pekko.apache.org/docs/pekko/current/typed/style-guide.html
[s21]: https://learn.microsoft.com/en-us/dotnet/orleans/grains/request-scheduling
[s22]: https://docs.dapr.io/developing-applications/building-blocks/actors/actors-overview/
[s23]: https://docs.dapr.io/developing-applications/building-blocks/actors/actors-timers-reminders/
[s24]: https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/
[s25]: https://asynkron.se/protoactor/supervision/
[s26]: https://learn.microsoft.com/en-us/azure/durable-task/common/durable-task-entities
[s27]: https://www.erlang.org/doc/system/ports.html
[s28]: https://docs.temporal.io/activity-definition
[s29]: https://docs.temporal.io/develop/typescript/workflows/message-passing
[s30]: https://restate.dev/blog/why-we-built-restate
[s31]: https://learn.microsoft.com/en-us/azure/architecture/patterns/circuit-breaker
[s32]: https://microservices.io/patterns/data/transactional-outbox.html
[s33]: https://docs.stripe.com/api/idempotent_requests
[s34]: https://martinfowler.com/bliki/CQRS.html
[s35]: https://learn.microsoft.com/en-us/azure/architecture/patterns/compensating-transaction
[s36]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/Aggregator.html
[s37]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/ProcessManager.html
[s38]: https://www.enterpriseintegrationpatterns.com/patterns/conversation/TryConfirmCancel.html
[s39]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/StoreInLibrary.html
[s40]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/IdempotentReceiver.html
[s41]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/CorrelationIdentifier.html
[s42]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/PipesAndFilters.html
[s43]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/PublishSubscribeChannel.html
[s44]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/RoutingTable.html
[s45]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/CompetingConsumers.html
[s46]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/DeadLetterChannel.html
[s47]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/MessagingGateway.html
[s48]: https://publish.illinois.edu/science-of-security-lablet/files/2014/06/Why-Do-Scala-Developers-Mix-the-Actor-Model-with-other-Concurrency-Models.pdf
[s49]: https://mbagherz.bitbucket.io/lab-correct-software/papers/akka-actor-bugs.pdf
[s50]: https://arxiv.org/html/1706.07372
[s51]: https://arxiv.org/html/1608.03321
[s52]: https://www.microsoft.com/en-us/research/publication/p-safe-asynchronous-event-driven-programming-2/
[s53]: https://rebeca-lang.org/assets/papers/2019/From-Requirements-to-Verifiable-Executable-Models-Using-Rebeca.pdf
[s54]: https://www.eventstorming.com/
[s55]: https://lamport.azurewebsites.net/tla/formal-methods-amazon.pdf
[s56]: https://learn.microsoft.com/en-us/dotnet/orleans/tutorials-and-samples/
[s57]: https://caitiem20.wordpress.com/2015/06/23/clients-are-jerks-aka-how-halo-4-dosed-the-services-at-launch-how-we-survived/
[s58]: https://www.erlang-factory.com/static/upload/media/1394350183453526efsf2014whatsappscaling.pdf
[s59]: https://discord.com/blog/maxjourney-pushing-discords-limits-with-a-million-plus-online-users-in-a-single-server
[s60]: https://www.microsoft.com/en-us/research/wp-content/uploads/2011/10/socc125-print.pdf
[s61]: https://learn.microsoft.com/en-us/dotnet/orleans/grains/transactions
[s62]: https://guug.de/veranstaltungen/ffg2003/papers/ffg2003-armstrong.pdf
[s63]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/CommandMessage.html
[s63a]: https://ferd.ca/the-zen-of-erlang.html
[s64]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/EventMessage.html
[s65]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/ReturnAddress.html
[s66]: https://learn.microsoft.com/en-us/dotnet/orleans/grains/grain-versioning/grain-versioning
[s67]: https://microservices.io/patterns/data/saga.html
[s68]: https://arxiv.org/pdf/1503.09052
[s69]: https://www.enterpriseintegrationpatterns.com/patterns/messaging/BroadcastAggregate.html
[s70]: https://learn.microsoft.com/en-us/azure/durable-task/common/durable-task-human-interaction
[s71]: https://docs.temporal.io/design-patterns
[s72]: https://proto.actor/protoactor/actors/
[s73]: https://arxiv.org/pdf/0909.1788
[s74]: https://docs.stripe.com/webhooks
[s75]: https://docs.restate.dev/develop/ts/durable-steps
