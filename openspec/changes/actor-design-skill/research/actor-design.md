# Designing systems with actors: research

Research for the planned `actor-design` skill, done on 2026-11-15. It is about the design level: which actors
exist, what each one owns, which messages they exchange, how they coordinate, how they fail, and how they meet the
outside world. It is not about how a runtime executes actors, and it is not about Cicada. The check against Cicada
is in [cicada-fit.md](cicada-fit.md).

A runtime property appears here only as a condition that changes a design decision: "if delivery can repeat a
message, then …".

## How to read the evidence

Two independent runs did this research and did not see each other's work. This document merges them.

- **Claude deep research.** 24 sources fetched, 115 claims taken from them with quotes, the 25 most central ones
  checked by three independent readers each. All 25 passed 3-0. Raw results: `raw/claude-verified.md`,
  `raw/claude-claims.md`.
- **Codex (gpt-6-astra).** One reader, 76 sources, no second check. It labels most of its own guidance
  "synthesis". Raw report: `raw/codex.md`.

Each statement carries a mark.

| Mark | Meaning |
| --- | --- |
| **V** | Verified. Three independent checkers read the source and agreed (Claude run). |
| **Q** | Quoted. One reader took it from the source with a direct quote; no vote (Claude run). |
| **C** | Codex read it in a source. One reader, no second check. |
| **I** | Inference or synthesis. It follows from the sources, but no source says it. |

Two marks on one statement mean that both runs found it independently. Source names in brackets, such as
[Armstrong], are listed in [Sources](#sources). Most guidance is one school's argued opinion, not a measured
result. See [Limits](#limits-of-the-evidence) and [where the two runs differ](#10-where-the-two-runs-differ).

## 1. Design principles

1. **Find the actors in the problem, not in the code.** Look for a thing that truly acts at the same time as
   others, a lasting business identity, or a resource that needs one authority. Classes and tables are not
   actors. **V** [Armstrong] [CF-rules] **C** [Orleans-BP]
2. **One rule, one owner.** Each rule that must hold at every step is enforced by exactly one actor. One command
   changes one actor. DDD aggregates help to find that boundary; making an aggregate an actor is a choice, not a
   law. **V** [Vernon] **C**
3. **Size an actor to its rule.** It owns the data that must be consistent together, and no more. **V** [Vernon]
4. **Messages are the public contract.** Define the requests, the outcomes and the rules of the conversation
   before the handlers. **V** [Armstrong] **Q** [Akka-IoT] **C** [Hewitt73]
5. **Only the owner changes its state.** Others ask for a decision or receive facts. Nobody changes the owner's
   state by another path. **Q** [TorresLopez] **C** [Armstrong] [Tasharofi]
6. **What you know about a partner is local knowledge, not its current state.** "Requested", "promised",
   "confirmed" and "unknown" are real states. Write them down. **C** [Helland-life]
7. **Across actors, agree by messages over time.** If several steps must all happen or be undone, that is a
   saga. Where the runtime offers a transaction across actors, say exactly what it covers. **V** [Vernon] [Sagas]
   **C** [Orleans-tx]
8. **Design every message for the moment the runtime's promise breaks.** If a message can repeat, name the work
   so that a repeat is recognised. If it can be lost, make its loss survivable. **V** [Helland-idem] **Q**
   [Akka-IoT] **C** [EIP]
9. **Sending is not doing, and a timeout is not a "no".** Only the application can say that work is done. A
   timeout means "unknown": the receiver may still finish the work, or start it later. **V** [Helland-idem]
   [Pekko-IP] **C** [Temporal-act]
10. **A refusal is data; a fault is a failure.** "Out of stock" is an ordinary reply. A fault ends the work, and
    someone else deals with it. **V** [Armstrong] **C** [Akka-intro]
11. **Design who recovers what, not just restarts.** Say who restores a service and which stable condition it
    promises afterwards. Decide first which part must be correct (the error kernel). **V** [Armstrong] [OTP-sup]
    **Q** [Hebert-guar] **C** [Proto-sup]
12. **Promise only what you can keep.** An actor in front of a thing that is expected to fail promises "I accept
    requests", not "I am connected". **Q** [Hebert-guar] **C**
13. **Your own atomic step does not reach the outside.** A local commit cannot include an e-mail or a payment.
    An outbox and a duplicate check at the receiver close two different gaps. **C** [Outbox] [Restate]
14. **Time belongs in the protocol.** Say when something expires, what happens to a late result, and who decides
    when completion races a deadline. **V** [Armstrong] **C** [Akka-IoT] [OTP-statem]
15. **Bound everything that grows.** State, pending work, kept results and subscriptions need limits and a way
    out. **C** [Hebert-anger]
16. **Use only the coordination the problem needs.** What a function can do stays a function. Transactions,
    streams and batch tools stay right for their jobs. **V** [Armstrong] [CF-rules] **C** [Dapr]
17. **No actor in the path of all traffic.** A single global actor is a bottleneck. **V** [CF-rules] **Q**
    [Orleans-BP]
18. **Check the bad traces.** Isolation does not remove message races, stale assumptions or "everyone waits"
    deadlocks. **Q C** [Bagherzadeh] [TorresLopez]

### Conditions that change a design

Do not import one framework's promises into a design. State which of these hold. **C**, with **Q** for the
second and fourth.

- **If messages can be lost:** choose an acknowledgement, a retry, an expiry, or accept the loss on purpose.
- **If messages can repeat:** protect each change that must not repeat with the identity of the logical work.
- **If one request excludes all others until it ends:** a slow dependency stops everything behind it, and a
  cycle of waiting calls deadlocks. [Orleans-BP] [Orleans-sched]
- **If other requests can run while a handler waits:** state can change across the wait. Check again after it,
  or reserve first. [CF-rules] [Orleans-sched]
- **If state, the incoming message's completion and outgoing messages commit together:** say so and rely on it.
  If not, design each gap.
- **If memory is lost when an instance ends:** say what can be rebuilt and which promises need storage.
- **If reminders survive and timers do not:** use the right one for each wait. [Dapr-timers]
- **If transactions or locks across actors exist:** state their scope. None of them undoes an outside effect.
  [Orleans-tx] [Azure-entities]

## 2. From problem to design: nine steps

The method is **I**: both runs built one, and this is the merge. It joins Armstrong's three steps (activities,
channels, messages), EventStorming for the story, aggregate design for the rules, and state tables for the
protocols. None of them hands you the actors. Each step ends with something written down.

| Step | Questions | Write down |
| --- | --- | --- |
| 1. The promise | What does the user get: success, refusal, cancellation, "unknown"? What must never happen? What must happen in the end? Which effects cannot be undone or are costly to repeat? Would a simpler design without actors do? | A short contract and the hard limits |
| 2. Who and what | Which facts occur, and what causes each? Who or what acts independently: customers, devices, sessions, jobs, processes? What is authoritative, derived, cached or only suspected? | A timeline of the domain |
| 3. Who owns each rule | Which data must be looked at together for each rule? Can one owner refuse a conflicting command without asking elsewhere? If a rule spans owners: whose job is it to make it consistent, and will a reservation or "in the end" do? | A map from each rule to its owner |
| 4. Identity and life | What makes two requests reach the same actor? Is the identity a lasting thing, a conversation, or one incarnation of a resource? Can a key be reused, and what happens to old messages then? Who creates it, who ends it, and what happens to its children, pending work and late replies? | Identity rules and a lifecycle sketch |
| 5. Responsibility cards | One sentence of purpose per actor. What does it alone decide and own? What does it know only by watching others? What does it not own? Which collections grow? | One card per actor (fields below) |
| 6. Protocols | Is each message a command, an event, a query or a reply? Who owns its meaning? What names the work, the conversation, the attempt, the reply address, the deadline? What does each acknowledgement prove: received, accepted, stored, reserved, done? What happens on a repeat, a stale reply, an invalid step? | A message list and a state table per actor |
| 7. Coordination and time | Draw success, timeout, repeat, reorder, cancel and a failed dependency. Does anyone wait while blocking the answer it waits for? Who records progress and picks the recovery? What can be reserved, and how do confirm, cancel and expiry race? What if a person never answers? | Sequence diagrams with the failure branches |
| 8. The outside | Who may decide an outside action, and who translates it? What names the effect across retries? Can the other side recognise a repeat, report status, or refuse a stale writer? If the effect happens before it is recorded, what is the next safe step? Which inbound events can repeat, arrive out of order or go missing? | An effect list, a gateway contract and a plan to reconcile |
| 9. Check and simplify | Can a short trace break a rule or leave accepted work with no owner? Which actors only forward calls and could be functions? Which actor mixes unrelated decisions and could be split? How do old callers, timers and conversations meet the new design? | Tested assumptions and counterexample traces |

**Card fields:** identity; purpose; what it decides; rules it protects; public messages; facts it emits; who it
works with; lifecycle; what it promises to keep; outside effects; deadlines; bounds; who recovers it.

**Is this an actor problem at all?** Ask it in step 1. Good fit: many small, independent units of state and
logic, each with its own identity, with interactive work. Poor fit: entities that must share memory, a few large
entities, global coordination or global consistency, queries across many actors, long batch computation, a pure
transformation, or a small CRUD application whose transactions are enough. **Q C** [Orleans-BP] [Dapr] **V**
[CF-rules]. A mixed system is fine: actors where ownership and conversation help, a read database, a
transactional part. Write down where authority changes hands. **C**

## 3. Pattern catalogue

Each pattern: the problem, the solution, what it costs, when not to use it.

### A. Finding the actors

**A1. Entity actor.** *Problem:* many things of one kind, each with its own state and rules. *Solution:* one actor
per thing, keyed by its identity; it holds the root and only the data that must stay consistent with it. *Costs:* a
rule that spans two things now needs messages. *Not when:* it would be one actor per table row, or it would split a
rule that cannot be split. **V** [Vernon] [CF-rules] **Q C** [Orleans-BP] [Dapr]

**A2. Activity actor.** *Problem:* calls, sessions, matches or jobs each run their own course. *Solution:* one
actor per activity, with its own protocol and its own end. *Costs:* cleanup must be explicit. *Not when:* it would
turn every noun or function into an activity. **V** [Armstrong] **C** [Armstrong-COP]

**A3. Coordination-unit actor.** *Problem:* several users act on one shared thing (a room, a document, a game).
*Solution:* one actor per shared thing; it is the single place where their actions meet. *Costs:* it serializes
everyone who uses that thing. *Not when:* the request rate is more than one actor can serve; split by a natural
key. **V** [CF-rules]

**A4. Resource owner.** See [G1](#4-actors-and-the-outside-world).

**A5. Conversation or process actor.** *Problem:* one request needs several replies, decisions or deadlines.
*Solution:* a conversation with its own identity holds the pending state and decides when it is complete. As a
short-lived child it replies and stops itself, and its protocol is private to its parent. Store it only if the
promise must survive a restart. *Costs:* one more participant; it must end on every path or it leaks; it needs a
deadline. *Not when:* one reply with a timeout is enough. **V** [Pekko-IP] **C** [EIP: Process Manager]

**A6. Parent and children for hierarchical data.** *Problem:* a workspace with projects, a server with matches.
*Solution:* the parent coordinates and keeps an index; each child owns its own state. Work on different children
runs in parallel, and listing them asks only the parent. *Costs:* two actors to keep in step. *Not when:* the
children have no life or rules of their own. **Q** [CF-rules]

**A7. Three trees, not one.** *Problem:* "parent" is used for three different relations. *Solution:* justify each
one separately: what contains what in the business, who creates whom, and who recovers whom. The recovery tree
follows dependencies, not the organisation chart or the message flow. *Costs:* more to draw. **C** [OTP-sup]
[Proto-sup]

**A8. Pure core inside an actor.** *Problem:* parsing, validation and calculation become messages. *Solution:*
functions and values inside the owner. A separate worker only for an independent job, for isolation, for
cancellation or for a resource. **V** [Armstrong] **C** [CF-rules]

**A9. Control-only singleton.** *Problem:* some authority is truly single and global. *Solution:* one actor for
the low-rate control work; the data path goes around it. *Not when:* requests pass through it. **V** [CF-rules]
**C**

**A10. Staged aggregation.** *Problem:* many actors report to one place. *Solution:* aggregate in levels, not into
one coordinator. *Costs:* results are late and approximate in between. **Q** [Orleans-BP]

### B. State and reads

**B1. Explicit state machine.** *Problem:* flags and callbacks allow impossible combinations. *Solution:* a table:
for each state and each message, the next state, the reply, the effects, and what happens to a message that does
not fit. *Costs:* modelling work. *Not when:* it would name every data value as a state. **Q** [Pekko-style] [P]
**C** [OTP-statem]

**B2. Explicit defer.** *Problem:* a message arrives that this state cannot handle yet. *Solution:* put it aside
on purpose, per state, and show that it is handled later. *Costs:* deferred messages can pile up or wait forever.
*Not when:* the message can be refused with a reply. **Q** [P] [Pekko-style]

**B3. Decide, then act.** *Problem:* the rules get lost among calls to dependencies. *Solution:* a pure decision,
"state and input give new state and intended actions"; the results of the actions come back as inputs. *Costs:*
it does not make an outside effect atomic. **I** (Codex), from [P] and [Outbox]

**B4. Ask the owner for a decision, not for data.** *Problem:* a caller reads, calculates and writes back.
*Solution:* send the intent: `TryReserve`, not `GetAvailability` and then an update. If a query is needed, return
a version with it. *Costs:* an answer is stale at once. *Not when:* never treat several queries as one snapshot.
**C** [Akka-IoT] [Vernon] **Q** [TorresLopez]

**B5. Reference by identity.** *Problem:* one actor needs another. *Solution:* hold the other's identity, never
its state. *Costs:* a view over several actors needs several lookups. **V** [Vernon]

**B6. Read model beside the actors.** *Problem:* search and reports cross owners. *Solution:* a derived model, fed
by the actors; say how fresh it is and how it is rebuilt. *Costs:* it lags, and it is more work. *Not when:* a
scarce thing is handed out from it, or the application is simple CRUD. **V** [Vernon] **Q** [Dapr] **C**
[Fowler-CQRS]

**B7. Bounded current state.** *Problem:* history, subscribers, results and pending work grow forever.
*Solution:* separate what a decision needs now from history and large content; set a retention rule. *Costs:*
retention changes behaviour. *Not when:* never drop the record that recognises a repeat while the repeat can still
come. **C** [Hebert-anger] [Stripe-idem]

**B8. Visible in-flight state.** *Problem:* a process is half done and others see the half. *Solution:* store
the in-flight part as data ("pending", "funds in transit"), not in the private variables of the process. **V**
[Sagas]

### C. Protocols

**C1. Commands ask, events tell.** *Problem:* generic "update" messages put the rules in the caller.
*Solution:* `ReserveSeat` asks for an action; `SeatReserved` reports a fact; `ReservationRejected` replies.
Incoming messages are commands; stored facts are named in the past tense. *Not when:* never hide work that must be
done as an event that nobody is responsible for. **Q** [Pekko-style] **C** [EIP: Command Message, Event Message]

**C2. Request and reply as two messages.** *Problem:* a sender needs an answer. *Solution:* the request carries
the reply address and a correlation id; the reply echoes the id and has its own name. *Costs:* pending state and a
rule for the reply that never comes. *Not when:* nobody needs the answer. **V** [Pekko-IP] [Armstrong] **Q**
[Akka-IoT] **C** [EIP: Correlation Identifier, Return Address]

**C3. Ask with a timeout.** *Problem:* the sender must know before it goes on. *Solution:* a request with a
deadline. Good for a single-answer query, for "I must know this was processed", for resending, and for limiting
open requests. *Costs:* one answer only; the timeout does not cancel the work; timeouts are hard to choose when one
ask causes more asks. *Not when:* several answers are needed, or asks would form a chain. **V** [Pekko-IP]

**C4. Acknowledgement that says what it proves.** *Problem:* "OK" could mean received, accepted, stored, reserved
or done. *Solution:* say which; report success only after the work is safe. *Costs:* one more message, which can
itself be lost. **Q** [Akka-IoT] [Helland-idem] **C**

**C5. Idempotent command.** *Problem:* a retry repeats a change. *Solution:* a stable key names the logical work
("process withdrawal XYZ"); the receiver applies it only if the key is new, returns the known result on a repeat,
and refuses the same key with a different intent. Keep the key together with the change it protects. *Costs:* the
receiver must remember keys, for a stated time. *Not when:* never make a new key per attempt, and never compare
payloads in place of a key. **V** [Helland-idem] **C** [EIP: Idempotent Receiver]

**C6. Three identities.** *Problem:* one id is used for everything. *Solution:* keep apart the logical operation
(for duplicates), the conversation (for correlation) and the attempt (for diagnosis). **C** [EIP] [Stripe-idem]

**C7. Tentative first message.** *Problem:* the first message to a partner may be retried and reach a partner
that does not remember it. *Solution:* make it trivial, read-only, or pending work that the partner only collects,
applies after a later round trip, and drops on timeout. **Q** [Helland-idem]

**C8. Own vocabulary.** *Problem:* a requester would have to accept another actor's reply types. *Solution:* the
receiver owns the meaning of its commands and the publisher the meaning of its facts; a requester translates
foreign replies into its own messages. *Not when:* several actors share one protocol on purpose; then put it in a
shared module. **V** [Pekko-IP] **Q** [Pekko-style] **C**

**C9. Public and internal messages.** *Problem:* timer ticks and wrapped replies are mechanics, not API.
*Solution:* keep them private, so that nobody outside can send them. **Q C** [Pekko-style]

**C10. Protocol as a state machine.** *Problem:* a valid message arrives in the wrong phase, and the mistake is
hard to trace. *Solution:* describe the whole conversation, with its failure branches: protocol state and message
give the allowed replies and the next state. Tag every message and say what happens to an unknown one. A violation
then names the faulty side. *Not when:* never take typed messages as proof that the order is right. **V**
[Armstrong] **Q** [P] [Maty] **C** [Session-actors]

**C11. Forward flow.** *Problem:* A waits for B waits for C, and the ones in the middle do not use the result.
*Solution:* pass the final reply address along and forward the work. Keep a middle step only if it decides
something or must recover something. *Costs:* routing becomes explicit. **V** [Pekko-IP] **C** [EIP: Pipes and
Filters, Return Address]

**C12. Self-contained message, claim check.** *Problem:* the receiver must call back for details, or the payload
is huge. *Solution:* put the intent, the facts and the versions in the message; refer to large unchanging content
by reference. *Not when:* never read "the latest row" as if it were the original request. **C** [EIP: Claim Check]

**C13. Result comes back as a message.** *Problem:* an outside call finishes later, on another thread.
*Solution:* turn its result into a message to the actor itself, with the context of the request. *Not when:* the
runtime lets the actor wait safely inside its turn. **Q** [Pekko-IP]

**C14. Evolve by meaning.** *Problem:* messages and conversations outlive deployments. *Solution:* keep the
meaning for old callers; version a changed conversation rule; translate at the border. *Not when:* never take "the
payload still parses" as "the behaviour is compatible". **I** (Codex), from [Orleans-ver]

### D. Coordination

**D1. Local knowledge of a partner.** *Problem:* owners cooperate without one atomic step. *Solution:* each
records, per relationship, what it requested, was promised, confirmed or cancelled, and what is unknown.
*Not when:* never treat an earlier reply as a lock on the partner. **C** [Helland-life]

**D2. Saga.** *Problem:* several steps in several actors must all happen, or be undone. *Solution:* a sequence of
local steps; on failure, run the compensation of each finished step in reverse order. A compensation is a new
business action ("cancel the reservation"), not a restore, and it can fail too. Put step boundaries at natural
divisions, often one per real-world action. *Costs:* no isolation: others see half-finished results. *Not when:*
the steps fit in one actor. **V** [Sagas] **C** [McCaffrey-sagas] [Azure-comp]

**D3. Forward-only saga.** *Problem:* compensations are hard to write. *Solution:* design the process so nobody
aborts it and every step succeeds if retried long enough. *Not when:* a user may cancel, or a step may fail for
good. **V** [Sagas]

**D4. One process actor per case (orchestration).** *Problem:* branching and recovery cannot be read from
scattered reactions. *Solution:* one process identity picks the next step and records what is owed; the entities
keep their own rules. *Not when:* never one global coordinator for unrelated cases. **C** [EIP: Process Manager]

**D5. Reactions to facts (choreography).** *Problem:* independent capabilities each react to a fact.
*Solution:* publish the fact; each subscriber owns its reaction and updates one actor in its own step; write down
who owns completion and the exceptions. *Costs:* the chain of causes is hidden. *Not when:* a long process that
must finish and that nobody could diagnose. **V** [Vernon] **C** [EIP: Publish-Subscribe] [Saga-pattern]

**D6. Reserve, then confirm or cancel.** *Problem:* undoing would be costly or impossible. *Solution:* reserve
first; the owner tracks the token, the expiry and the final step. *Costs:* it holds capacity; expiry races
confirmation. *Not when:* never assume several separate holds make one atomic group. **C** [EIP: Try-Confirm-Cancel]
**Q** [Helland-idem]

**D7. Allocated rights.** *Problem:* several owners must together stay under one limit. *Solution:* give each a
share that does not overlap; each spends only its share; transfers keep the total. *Costs:* unused shares are
stranded. **C** [Escrow]; using it with actors is **I**

**D8. Aggregator.** *Problem:* related replies must become one result. *Solution:* fix how replies are matched,
who the members are, and when it is complete: all, a threshold, the first good one, a deadline. *Not when:* never
wait for a number of answers that nobody fixed. **V** [Pekko-IP] **C** [EIP: Aggregator]

**D9. Scatter and gather.** *Problem:* independent parties can answer in parallel. *Solution:* choose the set,
send, collect under a deadline; keep apart "no data", "refused", "not available" and "timed out". *Not when:*
never send speculative duplicates of a command that cannot be undone. **C** [EIP: Scatter-Gather] [Akka-IoT]

**D10. Pipeline, routing slip.** *Problem:* stages vary per request. *Solution:* a fixed pipeline, or a route that
travels with the message; each stage has a stable identity and a place for exceptions. *Not when:* the route
depends on later results. **C** [EIP: Pipes and Filters, Routing Slip]

**D11. Publish and subscribe.** *Problem:* several reactions need one fact. *Solution:* define subscription,
replay, repeats and the life of a subscriber. Publishing is not completion of the reactions. *Not when:* one
accountable executor is needed. **C** [EIP: Publish-Subscribe]

**D12. Registry or index.** *Problem:* actors must be found. *Solution:* keep apart lookup, derived membership
and authoritative uniqueness; after the lookup, talk directly. *Not when:* never decide hard uniqueness from a
stale index. A lookup followed by a send is not atomic. **C** [Akka-IoT] **Q** [Isolation-trap]

**D13. Work distribution with admission.** *Problem:* independent jobs, and no limit on what is accepted.
*Solution:* define job ids, acceptance, assignment, completion, abandonment, retry and cancel; bound what is
accepted. *Costs:* after a failure, two workers may run the same job. **C** [EIP: Competing Consumers]
[Hebert-anger]

**D14. Waiting runs one way.** *Problem:* two actors that wait on each other deadlock. *Solution:* let
request-and-wait run in one direction only, down the hierarchy; the other direction uses one-way messages.
**Q** [Isolation-trap] [Orleans-BP] **C** [Orleans-sched]

### E. Time

**E1. Deadline as a step in the state machine.** *Problem:* waiting needs an outcome. *Solution:* keep the
deadline and a generation number; when the timeout arrives, look at the current state; say what happens to a
late result. A timer notice is not the deadline: it can be late, repeated or obsolete. **V** [Pekko-IP]
[Armstrong] **C** [Akka-IoT] [OTP-statem]

**E2. Stored reminder.** *Problem:* an obligation must survive a long wait or a restart. *Solution:* store the
obligation and its due time; use a scheduler whose promise matches; recognise a repeated or obsolete firing.
*Costs:* a due time is not an exact run time. *Not when:* losing it is harmless. **Q C** [Dapr-timers]
[CF-rules]

**E3. Each actor owns its own alarm.** *Problem:* per-entity scheduled work. *Solution:* the entity sets its own
alarm, only when there is work; its handler is safe to run twice. **Q** [CF-rules]

**E4. Debounce, throttle, batch.** *Problem:* inputs arrive more often than decisions are useful.
*Solution:* debounce replaceable updates, with a longest wait; throttle by an explicit admission rule; batch by
count, size or deadline, and keep the identity of each item. *Not when:* never drop ledger commands under a rule
made for replaceable readings. **I** (Codex), from [EIP: Aggregator]

**E5. Detector for the event that did not come.** *Problem:* something expected never arrives. *Solution:* track
the last observation and a generation; mark "suspect" after a deadline; say what suspicion allows. *Not when:*
never read a missed heartbeat as proof that the old party cannot act any more. **C** [Hebert-zen] [Akka-IoT]

**E6. Human task.** *Problem:* an approval takes days. *Solution:* a stored task with identity, assignee, the
version of the proposal, an expiry and a final outcome; commands to approve, reject, withdraw or reassign.
*Not when:* never accept an approval of an older version. **C** [Azure-human] [Temporal-msg]

**E7. Time bounds in the protocol.** *Problem:* "in the end" is not a contract. *Solution:* state the bound
("reply within T"). Armstrong proposes this; it is not common practice. **V** [Armstrong]

### F. Failure

**F1. Business failure as data.** *Problem:* a normal refusal looks like broken execution. *Solution:* explicit
outcomes for refused, conflicting and expired; say which ones allow a new request. *Not when:* never crash in
order to retry invalid input. **V** [Armstrong] **C** [Akka-intro]

**F2. Worker and supervisor.** *Problem:* error handling clutters the work. *Solution:* a worker does the work
and stops on what it does not understand; a different actor is told and decides. **V** [Armstrong]

**F3. Error kernel and risky leaves.** *Problem:* unstable work threatens valuable state. *Solution:* a small
trusted core that decides; independent risky workers; results checked before use. *Not when:* never keep state
that cannot be replaced only in a throw-away worker. **V** [Armstrong] **Q C** [Hebert-zen]

**F4. Recovery contract.** *Problem:* "restart" has no stated meaning. *Solution:* say what survives, what is
rebuilt, how open promises are settled, and when the failure goes up. Restart only the failed actor when its
peers are independent; all of them when they depend on each other; the failed one and those started after it when
the dependency follows start order. Limit restarts per time; past the limit the owner fails too. *Not when:* the
runtime has no supervising owner. **V** [OTP-sup] **Q** [Hebert-zen] **C** [Proto-sup]

**F5. Stable start.** *Problem:* peers rely on an actor that is "up". *Solution:* once started, it is stable
whatever happens outside; a thing that is expected to fail is not a condition of starting. *Not when:* the
dependency is a true precondition of the whole system. **Q C** [Hebert-guar]

**F6. The caller decides how much failure it tolerates.** *Solution:* a client actor reports the failure as
data; each caller chooses to ignore it, queue, or fail. **Q** [Hebert-guar]

**F7. One retry owner, with a budget.** *Problem:* every layer retries by itself, or forever. *Solution:* the
one that knows the logical identity, the deadline and whether the effect is safe to repeat does the retrying;
bound the attempts or the time. *Not when:* never retry blindly when the outcome is unknown and cannot be undone.
**C** [Temporal-patterns] **Q** [Orleans-BP]

**F8. Parking lot.** *Problem:* an input fails every time. *Solution:* keep its identity, content, error and
version; name who may repair and replay it; decide whether later work for the same entity may go on.
*Costs:* parked work is an open obligation, not a success. *Not when:* never use a runtime's dead-letter log as
the repair queue. **V** [Vernon] **Q** [Sagas] **C** [EIP: Dead Letter Channel]

**F9. Try something simpler.** *Problem:* the full task cannot be done. *Solution:* do a simpler task with
simpler code. **V** [Armstrong]

## 4. Actors and the outside world

Both runs agree on the starting point: an outside effect can finish before the caller has stored its result, and
no local commit can prevent that. **Q** [Akka-persist] **C** [Temporal-act] [Restate]

### Four kinds of authority

For each integration, name who holds each one. They can be one component or several. **I** (Codex)

1. **Business authority:** who may decide that the action should happen.
2. **Adapter:** who translates the intent into the other system's words.
3. **Outside authority:** which system settles whether it really happened.
4. **Recovery authority:** who resolves unclear or conflicting evidence.

A booking decides to charge. A payment adapter knows the provider's API. The provider's ledger settles whether
money moved. A reconcile step owns what is unresolved. The adapter must not invent a refund rule because it saw a
network error.

### Patterns

**G1. Resource owner.** *Problem:* a connection, device, account or external program is used by several parts.
*Solution:* exactly one actor owns it; all traffic to and from it goes through the owner; its end arrives at the
owner as an ordinary message. Keep the lasting identity of the resource apart from one connection or
incarnation. *Costs:* the owner serializes its use, and what it knows can lag behind reality. *Not when:* never
claim to be the only authority when other systems can act on the thing too. **Q** [OTP-ports] [Armstrong] **C**
[Akka-IoT]

**G2. Gateway, adapter, middle-man.** *Problem:* the outside's transport and words leak inward.
*Solution:* translate identities, units, operations and errors at the border, so the core sees only its own
terms. Make the gateway an actor when it holds state: a connection, a shared allowance, calls in flight, circuit
health. *Not when:* a stateless translation; that is a function. **Q** [Armstrong] **C** [EIP: Messaging Gateway]

**G3. Client that serves while disconnected.** *Problem:* a remote service is down. *Solution:* "disconnected"
is a valid state; the client still accepts requests and answers with an explicit error; it reconnects by itself,
with backoff. **Q C** [Hebert-guar]

**G4. The same key on every attempt.** *Problem:* an outside effect may be sent twice. *Solution:* create one
stable key for the logical effect before the first uncertain attempt, and keep it. Use a business value when one
exists (the order id); otherwise the identity of the process plus the step. Create it as close to the origin of
the request as you can. An attempt number is for diagnosis; it does not replace the key. *Costs:* the key is a
contract: check its scope, how long the other side remembers it, and what it does with a changed payload. Stripe,
for one, may forget a key once it is 24 hours old. **Q** [Temporal] **C** [Stripe-idem]

**G5. Effect record and reconciliation.** *Problem:* what was intended and what happened outside disagree.
*Solution:* store the intended operation, the attempts, the evidence and the outcome; keep "unknown" as a real
state with an owner; compare the record with independent evidence on a trigger and a schedule; let the result
come back as a normal message. After recovery, re-issue every effect that was not acknowledged. *Costs:* some
effects cannot be resolved without a person. *Not when:* never a single "sent" flag. **Q** [Akka-persist] **C**
[Temporal-act] [Restate] [Quicksand]

**G6. Outbox and inbox.** *Problem:* a state change and its notice come apart on failure. *Solution:* store the
change and the outgoing intent together, and deliver later (outbox); at the receiver, store "seen" together with
its own change (inbox). *Costs:* delivery can repeat. *Not when:* never use them to claim "exactly once" for an
effect in a third system. **C** [Outbox] [EIP: Idempotent Receiver]

**G7. At most once, then compensate.** *Problem:* an effect that cannot be made safe to repeat. *Solution:* try
it once; on an unknown outcome take another path: a compensation or a person. Some effects cannot be undone, only
countered ("a second letter"). **Q** [Temporal] **V** [Sagas]

**G8. Check around the call.** *Problem:* if other messages can run while the actor waits on an outside call, the
state may change under it. *Solution:* read a version before the call and check it before the write. *Not when:*
the actor handles one message to the end. **Q** [CF-rules] **C** [Orleans-sched]

**G9. Quota owner and circuit state.** *Problem:* callers share a limited allowance, or a dependency is
unhealthy. *Solution:* the scope of the quota actor matches the real allowance (account, credential, device,
tenant). The circuit is state about the dependency's health: closed, open, half-open with controlled probes.
Refuse, defer or degrade on purpose. A business refusal does not count as a dependency failure. *Not when:* never
assume that limits per user enforce a limit of the provider. **C** [Azure-CB] [McCaffrey-Halo] **Q**
[Isolation-trap]

**G10. Risky foreign code as a watched worker.** *Problem:* a library in another language may crash.
*Solution:* run it as its own program, watched by an actor, with a high tolerance for failure; move data to a
stabler actor as it becomes valuable. Code loaded into the runtime itself has no such isolation. **Q**
[Hebert-zen] [OTP-ports]

### The effect may have happened, but its record is missing

A failure after the effect and before the record leaves the same local evidence as a failure before the effect.
Swapping the order does not help. **Q** [Akka-persist] **C**

| Order | What a crash in between leaves |
| --- | --- |
| Act, then record | The effect happened and nothing says so. A retry does it again. |
| Record, then act | The record says "done" and the effect may not have happened. Without a re-issue step it is lost. |

So "unknown" is a real outcome. It needs an owner and a way forward. Go through these in order, and check the
other system's actual contract, not the name of the HTTP method. The table is **I** (Codex), with **Q** [Temporal]
for rows 2, 4 and 8.

| # | If | Then | Watch for |
| --- | --- | --- | --- |
| 1 | The effect and your record can commit in one real transaction | Put both in it | It covers only what takes part. An HTTP call inside a transaction is not part of it. |
| 2 | The other side accepts an idempotency key | Repeat or ask, with the same key and the same intent | Scope, retention, changed payloads, two attempts at once |
| 3 | It supports "create with this id" or "change only if in state X" | Retry with the same id or condition, then read the result | Do not overwrite a later, legitimate state |
| 4 | It can report the status of an operation | Ask by operation id and adopt the answer | "Not found" proves nothing while the first attempt may still land. Stay "unknown". |
| 5 | The result shows up in a feed or a query, later | Reconcile within a stated time, with a duplicate check | An empty read is weak evidence |
| 6 | It supports reserve and confirm | Recover the reservation, then confirm or cancel | Expiry races |
| 7 | A repeat is harmless here | Retry | "Set to X" is not harmless if a stale retry overwrites a newer X. Use a version. |
| 8 | A duplicate can be found and compensated | Record it and run a named compensation | You accept a duplicate for a while |
| 9 | None of the above | Stop retrying. Keep the evidence. Show "unknown". Hand it to someone with authority. | The business may need another provider or a weaker promise |

In every case: keep the pending effect visible in state, and keep effects out of code that runs again on
recovery or replay. **Q** [Akka-persist]

### Device twins

A device owner keeps apart: the **desired** state, the last **reported** state, where and when each observation
came from (source, time, sequence, incarnation), the **pending commands**, how sure it is (connected, suspect,
offline), and the **business lifecycle** (registered, enabled, retired). "Command received" does not prove that
the valve closed. A reconnect must not make an old reply fit a new command: carry a generation. **I** (Codex),
from [Akka-IoT]

### OS processes and child programs

In Erlang, the process that opens a port owns it; all traffic goes through the owner; when the owner ends, the
port closes. The external program is only cleaned up if it exits when its port closes: the end of the actor does
not kill a process tree. Handing a port to a new owner does not hand over the failure link. **Q** [OTP-ports]
**C**

A design for an actor that owns a child program says: **I** (Codex)

- how a job is identified before the launch;
- the states: starting, ready, busy, stopping, exited, unknown;
- how requests and output are matched, and how old output is told from a restarted program's output;
- whether the exit status means "the work is done" or only "the program ended";
- who handles error output, malformed output, a missing "ready" and stalled progress;
- a graceful stop, then a bounded escalation;
- whether grandchildren can outlive it, and which files and outside effects remain;
- whether recovery reconnects to the running job or starts a new one.

### Inbound events: webhooks, polling, change feeds

At the door: authenticate the source, translate its identity into yours, keep the event id and the evidence, and
keep apart "accepted and stored" from "the reaction is done". Stripe documents that webhooks can repeat and can
arrive out of order. **C** [Stripe-webhooks]. The rest is **I** (Codex):

- One event delivered twice: recognise its event id.
- Two different events about the same change: recognise the change, too.
- Out of order: use the source's version, or read the authoritative state again.
- A polling cursor: is "advance the cursor" atomic with "keep what was found"? If not, prefer a safe replay over
  a silent gap.
- A missed webhook: can polling repair it, or must a person look?
- A callback before registration or after the end: keep, refuse or reconcile.
- An event may be a snapshot, a change, or a hint to fetch. Each needs a different protocol.

### Files, hardware, people, clocks, randomness

**I** (Codex), with [Restate-steps] and [Azure-human].

- **Files:** a path is not a content identity. Write to a unique name, check, then publish a stable reference.
- **Hardware:** a retry must be safe for the physical action. "Dispense one item" is unlike "set the target
  temperature".
- **People:** a second click or a late callback must not approve another revision.
- **Clocks:** say whose time decides an expiry. A timeout schedules a second look; it does not observe that the
  other side stopped.
- **Randomness:** draw once and store the result. Do not draw again after a recovery.
- **E-mail:** "the provider accepted it" is not "the user read it".

### What an actor promises to its users

A template, to be replaced with the real choices. **I** (Codex)

> Commands for this booking are checked against its current version. "Accepted" means the operation and its
> follow-up are stored. Repeating the same operation id with the same intent returns its current or final result
> during the retention period. "Completed" means the named outside evidence is recorded. A timeout does not mean
> cancellation. Unknown operations stay queryable and owned until they are resolved or handed to a person.

## 5. Do's and don'ts

**Finding the actors**

- Do name what each actor decides. A noun alone does not justify an actor. **C**
- Do start from the things that act at the same time in the problem. **V**
- Do size an actor to its rule: the root and the data that must be consistent with it. **V**
- Do merge two actors that talk to each other all the time. **Q C** [Orleans-BP]
- Do split an actor when unrelated actions get in each other's way. **V**
- Do keep the lasting id of a resource apart from the id of a connection or session. **C**
- Don't make an actor for what a function call can do. **V**
- Don't put all traffic through one actor. A singleton for a truly single, low-rate authority is fine. **V C**

**State**

- Do keep one owner for each rule. **V**
- Do hold other actors by identity only. **V**
- Do keep apart what is authoritative, what was observed and what is a projection. **C**
- Do store half-done work as visible data. **V**
- Do bound histories, subscribers and pending work. **C**
- Don't allow a later change only on the strength of an earlier query. **C Q**
- Don't let anything else write an actor's records. Shared storage is not shared authority. **C Q**
- Don't add event sourcing or a read model just because there are actors. **C**

**Protocols**

- Do write down every message before the code. **V**
- Do use verbs of the domain and explicit outcomes. **C**
- Do give every request a reply address and a correlation id. **V**
- Do name the work in the message, so a repeat is recognised. **V**
- Do say, for every state, what happens to every message. **Q C**
- Do keep internal messages out of the public protocol. **Q C**
- Don't depend on the last message arriving. **V**
- Don't treat a timeout as "it did not happen". **V**
- Don't rely on order between different senders, or on two of your messages being handled back to back. **Q**
- Don't use fire-and-forget for work whose success must be known. **Q**
- Don't turn every exchange into an ask. Each reply must be matched, expired and recovered. **C**

**Coordination**

- Do ask "whose job is it to make this consistent?" before choosing immediate or eventual. **V**
- Do write a compensation as a new business action. **V**
- Do write down who owns completion when reactions are spread out. **C**
- Do let waiting calls run in one direction only. **Q**
- Don't change several actors in one step; if you must, the boundaries are probably wrong. **V**
- Don't cure a call cycle by letting requests interleave, unless every rule is checked again after each wait. **C**

**Time and failure**

- Do keep the deadline apart from the timer notice; the notice can be late. **C**
- Do refuse an old timer generation; an old timeout must not end a new conversation. **C**
- Do sort every failure into "foreseen outcome", "cannot right now" or "unknown". **V C**
- Do say what a restart means for state and for open promises. **C**
- Do check outside data once, where it enters the system. **Q** [Armstrong]
- Don't retry at every layer. **C**
- Don't park work without an owner and a rule for replay. **C**
- Don't use the supervisor as a retry timer against an outside service. **Q** [Hebert-guar]
- Don't crash away data that matters. **I**

**The outside world**

- Do give each outside resource one owner. **Q**
- Do send the same key on every attempt of an outside effect. **Q C**
- Do treat "accepted" and "completed" as two claims. **C**
- Do compare what you want with what you observe; remembering a value does not make it true. **C**
- Don't put an effect in code that runs again on recovery or replay. **Q**
- Don't use "does it exist? then create it" in place of a key. **Q** [Temporal]
- Don't assume that ending an actor undoes a payment, kills every child program or takes back an e-mail. **C**

## 6. Anti-patterns

A symptom is a reason to look, not proof that the design is wrong.

| # | Anti-pattern | Symptoms | Fix | Evidence |
| --- | --- | --- | --- | --- |
| 1 | God actor | Unrelated user actions block or fail each other; it handles several subjects; it changes for every feature | Split by independent decisions and lifetimes; keep each rule whole | **V** [Vernon] **C** |
| 2 | Rule split over actors | Two actors each approve changes that together break a limit; code reads one actor, then writes another | Merge the authority, allocate rights, reserve, or design the saga | **V** [Vernon] **C** |
| 3 | Too-fine actors | Actors that only pass calls on; lifetimes that always coincide; no rule of their own | Values and functions, or merge | **V** [Armstrong] **C** |
| 4 | Anaemic CRUD actors | Getters and setters; callers read, calculate and write back | Commands and checks at the owner; or plain CRUD if there is no behaviour | **C** |
| 5 | Singleton in the data path | A global counter, limiter, manager or registry that every request passes | One actor per natural key; staged aggregation; talk directly after the lookup | **V** [CF-rules] **Q C** [Orleans-BP] |
| 6 | Chatty actors | Several round trips for one decision; fetching small fields again and again | One message with what is needed and the intent; or merge | **Q C** [Orleans-BP] |
| 7 | Actors used as plain objects | Calls that look like property access; blocking waits; an exception expected at once | Coarse operations with explicit outcomes. The fault is the reasoning, not the syntax. | **C** [Orleans-BP] **Q** [Bagherzadeh] |
| 8 | Ask chains | An ask causes an ask causes an ask; each layer adds its own timeout and retry | Forward flow; state per conversation; one process actor | **V** [Pekko-IP] **C** |
| 9 | Call cycle | A waits for B while B waits for A; hangs or cascading timeouts | Waiting in one direction only; one-way messages back | **Q** [Orleans-BP] [Isolation-trap] **C** [Orleans-sched] |
| 10 | Blocking on a child's reply | A parent waits on its child; the child fails and never replies; the parent cannot restart it | Never block inside a handler; take the reply as a message | **Q** [Bagherzadeh] |
| 11 | Actor state used in a callback | A reply goes to the wrong actor or to nobody; state changes "by itself" | Copy what the callback needs first; bring the result back as a message | **Q** [Bagherzadeh] [Pekko-IP] |
| 12 | Shared state through a side channel | Two actors update one record; a script goes around the rules; a registry is read and then used | One owner for each record; repairs go through the owner | **Q** [TorresLopez] [Isolation-trap] **Q C** [Tasharofi] |
| 13 | Check, then act, over two messages | "Is the key free?" then "store it", and another client stores in between | One message that checks and acts in the owner's single step | **Q** [TorresLopez] [Temporal] **C** |
| 14 | Mutable data in a message | The sender changes an object after sending it; the receiver hands out its own collection | Immutable values or copies | **C** |
| 15 | Reliance on order or timing | A sleep "fixes" a test; events from two sources are assumed in order; a late reply updates new work | Versions, generations, and a rule for the step that does not fit | **Q** [TorresLopez] [Akka-IoT] **C** [Bagherzadeh] |
| 16 | Holes in the state table | A message arrives in a state with no handler for it | A full table: handled, deferred on purpose, or refused | **Q** [TorresLopez] [P] [Maty] **C** |
| 17 | Waiting for a message nobody sends | Nothing is blocked, yet nothing moves | A deadline on every wait; review the protocols together | **Q** [TorresLopez] [Maty] |
| 18 | Fire-and-forget for important work | The sender "can never be sure if the message was processed" | An acknowledgement after the work | **Q** [Akka-IoT] |
| 19 | Timeout read as failure | A resource is freed or a compensation runs while the first action still goes on | "Unknown" as a state; retry with the same key, or ask | **V** [Helland-idem] [Pekko-IP] **C** |
| 20 | "Exactly once" by wishing | "Mark sent, then call" or "call, then mark done", with no state for doubt | An effect record, a key at the receiver, reconciliation, or a person | **C** **Q** [Akka-persist] |
| 21 | Outbox as a cure-all | The outbox is given as the reason a payment or e-mail happens once | It protects only your own state and notice; look at the receiver separately | **C** [Outbox] |
| 22 | Effect in replayed code | An e-mail is sent again at every restart | Effects only in the step that decides | **Q** [Akka-persist] |
| 23 | Crash as business logic | "Not found" or "insufficient funds" crashes and restarts | Outcomes as data; faults only for broken assumptions | **V** [Armstrong] **C** |
| 24 | Restart taken for durability | Success is reported before the state is safe; after recovery the state is empty | Say what "accepted" needs stored, and what is rebuilt | **C** |
| 25 | Poison message loop | One input crashes a valuable owner again and again | Park it with an owner, or stop on purpose; never skip a critical event in silence | **C** **Q** [Sagas] |
| 26 | Outside service as a start condition | The system does not start because a remote service is down | "Disconnected" is a valid state; reconnect later | **Q C** [Hebert-guar] |
| 27 | Supervisor as retry loop | Restarts use up the limit while a service is down, and the tree stops | Retry with backoff inside the actor; keep restarts for faults | **Q** [Hebert-guar] |
| 28 | Thread-style error handling | A try/catch around a send or a create, which never catches anything | Failure arrives as a message to the owner; design that path | **Q** [Bagherzadeh] |
| 29 | Leaked or dead helpers | Conversation actors that never stop; messages to a child that already stopped | End a helper on every path, with a deadline | **V** [Pekko-IP] **Q** [Bagherzadeh] |
| 30 | Unbounded state | Lists of finished ids, subscribers, history or pending requests that only grow | Retention rule, summaries, archive, admission limit | **C** [Hebert-anger] |
| 31 | Actor that answers everyone's reads | One actor holds a routing table or config that thousands read through it | A copy or a read model for readers; the owner for writes | **Q** [Isolation-trap] |
| 32 | Actors for pure computation | A chain of actors that only transform input | Functions, or a dataflow tool | **V** [Armstrong] **C** |
| 33 | Twin that claims to know | The desired state is shown as the measured state; a reconnect wipes the device's history | Keep desired, reported, time and confidence apart | **C** |
| 34 | Mixing actors with other concurrency | Locks, shared memory or futures next to actors | Keep the actor part pure, or draw a clear border | **Q C** [Tasharofi] |

Two warnings against over-reading the table, both from Codex. A shared SQL database is not #12 by itself: actors
can share storage while each owns its own records; the fault is a write that goes around the owner. And an
asynchronous call that looks like a method call is not #7 by itself: Orleans chose that style on purpose.

How often some of these happen, in the one study that counted (186 Akka bugs): logic 19%, races 15%, confusion
about the API 14%, lifecycle 12%, plain programming 12%, messaging patterns 10%, thread-style thinking 8%. Of the
races, 30% came from one mistake: using the actor's "sender" inside a callback. **Q** [Bagherzadeh]

## 7. Checking a design

### Review checklist

Answer yes, no, or "does not apply, because …". For each yes, name where the design or the code shows it. The
number in brackets is the pattern or anti-pattern behind the check.

**Actors and ownership**

1. Can you say in one sentence what each actor decides, does or owns? (A1, #1, #3)
2. Does each rule that must always hold have exactly one owner, or a written way to coordinate? (#2)
3. Do unrelated user actions stay out of each other's way? (#1)
4. Are keys canonical, scoped to the tenant where needed, and stable across retries? (step 4)
5. Is it written down what happens on key reuse, deletion and a message that arrives after the end?
6. Are containment, creation and recovery each justified by themselves? (A7)
7. Are functions and values used where an actor adds nothing? (#3, #32)
8. Is each singleton's scope a truly single authority, and is it out of the data path? (#5)

**State**

9. Can only the owner change its state? (#12)
10. Are messages immutable values? (#14)
11. Are observed and projected data kept apart from authoritative state? (#33)
12. For every state: is every message handled, deferred on purpose, or refused? (#16)
13. Is no change allowed on the strength of an older read alone? (#13)
14. Are state, history, subscribers and pending work bounded? (#30)
15. Does retention keep the records that recognise a repeat for as long as a repeat can come? (B7)
16. Do read models say how fresh they are and how they are rebuilt? (B6)

**Protocols**

17. Is every public message clearly a command, an event, a query or a reply? (C1)
18. Does every acknowledgement say what it proves? (C4)
19. Are the operation, the conversation and the attempt identified apart? (C6)
20. Is it defined what happens on a repeat, and on the same key with another intent? (C5)
21. Does every request carry a reply address and a correlation id? (C2)
22. Is a late or unknown reply handled on purpose? (#19)
23. Can internal messages not be sent from outside, or is that harmless? (C9)
24. Does nothing depend on the last message arriving, on order between senders, or on two messages being
    handled back to back? (#15)
25. Is each ask needed for its result? (#8)
26. Is every protocol version in use supported or migrated? (C14)

**Coordination and time**

27. Is the graph of waiting free of cycles? (#9)
28. If requests can interleave: do the rules still hold after every wait? (G8)
29. Does no single step change more than one actor? (#2)
30. Does someone own completion and the exceptions of every process? (D4, D5)
31. Are compensations identified, safe to repeat and recoverable? (D2)
32. Is half-done work visible as data? (B8)
33. Are the races between expiry, confirm and cancel of a reservation defined? (D6)
34. Is it fixed, per conversation, who answers and when the answer is complete? (D8)
35. Are fan-out and work in flight bounded? (D13)
36. Does each deadline have an owner and a rule for the late result? (E1)
37. Are old timers and reminders recognised and ignored? (E1, E2)
38. Must each timer survive a restart, and does it? Is its handler safe to run twice? (E2, E3)
39. Can a human task expire, be withdrawn, and refuse an approval of an older version? (E6)
40. Is a missed heartbeat treated as suspicion, not as proof? (E5)

**Failure**

41. Are refusal, "cannot right now" and "unknown" three different outcomes? (F1, #23)
42. Is it written what a restart restores? (F4, #24)
43. Is the part that must be correct named, and is it free of risky work? (F3)
44. Is escalation bounded and does it end at someone? (F4)
45. Does exactly one place retry each logical operation? (F7)
46. Is work that can never succeed parked with an owner? (F8, #25)
47. Does nothing use restarts as a retry timer? (#27)

**The outside world**

48. Does each outside resource have exactly one owner? (G1)
49. Does each outside effect have a stored identity that is the same on every attempt? (G4)
50. Has the gap "effect done, record missing" been worked through for each effect, and does the chosen row of
    the table match what the other side really promises? (section 4)
51. Is the outbox's promise kept apart from the effect's promise? (#21)
52. Is no effect in code that runs again on recovery or replay? (#22)
53. Can a late success be reconciled after a timeout or a cancel? (#19)
54. Is it written what ending the owner does not clean up: child programs, files, payments? (G1)
55. Are repeated, reordered and missed inbound events handled? (section 4)
56. Do the scopes of quota and circuit match the real dependency? (G9)
57. Does the system start and keep serving when an outside service is down? (G3, #26)
58. Are the promises to callers written in the words of the domain? (section 4)

### Properties worth checking

**I** (Codex). Say what you assume about the environment before you claim that something happens in the end.

- **Never:** a seat confirmed for two bookings; one key bound to two intents; a confirmed outcome turned back by
  an old reply; a cancelled case whose late effect vanishes from the record; an old generation allowed to act.
- **In the end:** every accepted operation reaches a final state or an owned escalation; an expired reservation
  becomes free unless confirmation won; every parked operation can be found by its recovery owner.

Generate repeats, the losses the delivery model allows, reorderings, time steps, crashes before and after every
commit and every outside effect, old generations, invalid input, and the same key with a different payload.

### Tools, and what each can and cannot do

| Tool | Good for | Limit | Evidence |
| --- | --- | --- | --- |
| State tables and sequence traces | Missing steps, open obligations and cycles, cheaply | People miss combinations | **Q** [P] **C** |
| Property tests against a model | Generated message histories, checked against a simple reference | A sample; the generator may leave out the real fault | **C** |
| TLA+ | Rules that must never break and things that must happen, on small adversarial setups | The model must match what is built | **C** [TLA-AWS] |
| P | Actors as communicating state machines; finds unhandled events and events deferred forever. Model the outside as an actor that may send anything its protocol allows. | The outside must be modelled; exploration is bounded | **Q** [P] **C** |
| Rebeca | Actor models with timing, traced back to requirements | Finite models and timing assumptions | **C** [Rebeca] |
| Session types, monitors | Who may send what, in which order | Not payloads, crashes, outside effects or business rules. Two protocols that are each free of deadlock can deadlock together. | **Q** [Maty] **C** [Session-actors] |

Static analysis and testing tools find communication deadlocks and protocol violations. They do not find
livelocks or "everyone waits" deadlocks. **Q** [TorresLopez]. A design that mixes actors with other concurrency
cannot use most of these tools. **Q** [Tasharofi]

### Worked examples worth reading

**C**, with what to take from each.

- **Akka IoT guide:** device ownership, registration, and a query as its own conversation that tells "no reading",
  "not available" and "timed out" apart. It leaves the physical protocol out. [Akka-IoT]
- **Halo on Orleans:** ownership keyed by player and game. The launch report adds: your own clients can produce
  harmful traffic. [Orleans-paper] [McCaffrey-Halo]
- **Discord:** session and guild processes as lifecycle boundaries; later the fan-out moved from the guild to a
  sender process. A boundary can be right and still need to shed work. [Discord]
- **Rebeca door control:** requirements become state models and checked properties. [Rebeca]
- **WhatsApp:** evidence of practice at scale; little about how to cut a domain into actors. [WhatsApp]
- **Codex's own exercise:** seats, payment and tickets, with the actors, the messages and a state table for the
  payment. It is in `raw/codex.md`, section 3 H.

## 8. Changing a design

**I** (Codex), from [Vernon] and [Orleans-ver].

**Split when** an actor holds unrelated rules, separate business lifetimes, independent outside resources or
separate recovery needs, or when one conversation's pending state hides the others.

**Merge when** most work needs calls both ways, the two must always change together, a child has no authority or
life of its own, or the messages only imitate field access.

Moving code is not enough: a split turns a local decision into a protocol, and a merge changes what can fail and
be cancelled separately. A safe plan:

1. Write down the old contract and its rules.
2. Name who has the authority before, during and after.
3. Keep or translate the identities and the records that recognise repeats.
4. Decide where unfinished conversations and old replies go.
5. Move state and open obligations in a way that rules out two writers.
6. Keep a tombstone or a redirect for late work.
7. Check traces that mix old and new before the old protocol goes.

## 9. Where the frameworks disagree

These are differences in what each one assumes or stresses. Not all are contradictions.

| Question | One side | Other side | Evidence |
| --- | --- | --- | --- |
| What deserves an actor? | Armstrong: each thing that truly acts at the same time, and each resource | Orleans, Dapr: many independent keyed entities. Cloudflare: units of coordination, capped by request rate. Vernon: the rule decides the size. | **V** [Armstrong] [Vernon] [CF-rules] **Q C** [Orleans-BP] [Dapr] |
| Everything an actor? | The 1973 formalism can express functions and data as actors | Production guidance: plain functions where nothing needs coordination | **C** [Hewitt73] [CF-rules] |
| How small? | Small independent actors give finer failure boundaries | Small chatty ones should be merged | **Q C** [Hebert-zen] [Orleans-BP] |
| Who owns an actor's life? | Erlang/OTP, Akka, Proto.Actor: a supervising parent starts, stops and restarts it | Orleans, Dapr, Durable Objects: an identity outlives its running instance, and nobody supervises it. Fewer lifecycle bugs. The business still needs create and delete in the model. | **V** [OTP-sup] **Q** [Bagherzadeh] **C** [Orleans-paper] |
| What is promised about delivery? | Akka, Orleans: at most once. Design for loss; retry from the edge. | Durable execution: unfinished work is retried. Design for repeats. | **Q** [Akka-IoT] [Orleans-BP] **C** [Temporal-act] [Restate] |
| One message at a time? | Orleans: yes by default, so call cycles deadlock | Re-entrant modes and Durable Objects let other requests in while a handler waits on outside I/O | **Q C** [Orleans-BP] [Orleans-sched] [CF-rules] |
| Request and reply | Message-first: explicit replies and forward flow | Orleans: asynchronous calls that look like method calls, on purpose | **V** [Pekko-IP] **C** [Orleans-BP] |
| What is the public API? | Akka Typed: the message types | Erlang: functions that hide the messages | **V** [Pekko-IP] [Armstrong] |
| How is a protocol checked? | Akka Typed: static types | Armstrong: a run-time contract checker. Research: session types. | **V** [Armstrong] **Q** [Maty] |
| Where is the reply address? | Akka Typed: an explicit field | Classic Akka: an implicit "sender" | **V** [Pekko-IP] |
| Consistency across actors | Helland, Vernon: no shared transaction; small boundaries and agreement over time | Orleans: opt-in ACID transactions. Azure entities: durable critical sections, which are not a rollback. | **V** [Vernon] **C** [Helland-life] [Orleans-tx] [Azure-entities] |
| Who runs a business process? | Akka, Erlang: an actor | Dapr, Temporal, Azure: a workflow above the actors, built for long waits | **Q** [Dapr] **C** [Temporal-msg] [Azure-human] |
| Long work | Dapr, Orleans: do not block an entity with long work | Temporal, Azure workflows: long waits and approvals are the model | **C** [Dapr] [Azure-human] |
| What survives a restart? | Timers in memory are lost | Reminders and alarms are stored, and may fire twice | **Q C** [Dapr-timers] [Pekko-IP] [CF-rules] |
| Effects after a stored change | Akka persistence: at most once; re-issue by hand after recovery | Temporal, Restate: the unfinished step runs again; make it idempotent | **Q** [Akka-persist] **C** [Temporal-act] [Restate] |
| Sagas | 1987: long database transactions, one executor, compensations | Today: across services and trust borders. Do not import the paper's database assumptions. | **V** [Sagas] **C** [McCaffrey-sagas] |
| Queries | Ask the owner: one owner's answer | A separate read model, which Fowler warns is extra complexity | **C** [Akka-IoT] [Fowler-CQRS] |
| Defensive code | Armstrong: do not program defensively | The same rule set: check data once at the border | **V Q** [Armstrong] |
| Reads of widely used data | Through the owning actor | A read path outside the actor. Whether the bottleneck is the model's fault or "role confusion" is disputed. | **Q** [Isolation-trap] |

## 10. Where the two runs differ

The runs do not contradict each other. They differ in what they read and in tone.

| Point | Claude run | Codex run |
| --- | --- | --- |
| Method | 24 sources; 25 claims voted 3-0; the rest quoted without a vote | 76 sources; one reader; most guidance labelled synthesis |
| Strongest on | Finding the actors, protocols, sagas, failure, the bug studies | The outside world, time, coordination patterns, the design method, checking |
| Helland | Read "Idempotence Is Not a Medical Condition" (from an archive); could not load "Life Beyond Distributed Transactions" | The reverse |
| Ask | Its sources list the costs of ask and warn about chains | Ask is a supported idiom; the fault is the chain and the hidden waiting |
| Singletons | An anti-pattern when all traffic passes | Do not reject them all; justify the scope |
| Shared database | Side channels cause races (from the bug studies) | Shared storage is fine when each record has one owner |
| Transactions across actors | Its sources assume none | Some runtimes offer them; state the scope |
| Hierarchy | Parent and children follow the data (Cloudflare) | Containment, creation and recovery are three relations |
| Dead letters | A place for stuck work is needed | A runtime's dead-letter log is not that place by itself |

## Limits of the evidence

- **Verified is narrow.** The 25 verified claims come from seven sources: Armstrong's thesis, the OTP
  supervision guide, Vernon, the Sagas paper, Helland on idempotence, the Pekko interaction patterns and
  Cloudflare's rules. They cover finding the actors, protocols, sagas and failure.
- **The rest has one reader.** **Q** and **C** statements were each read by one model in one source. Where both
  runs agree, that is two independent readers, not a vote.
- **Much is synthesis.** The nine steps, the effect table, the device and child-program lists, the checklist and
  the split and merge plan are proposals by the runs. No study tested them.
- **Three transfers are inference.** Aggregate to actor (Vernon never mentions actors). Saga to a process over
  actors (the paper is about one database). Service dialog to actor conversation (Helland).
- **Argued, not measured.** Armstrong, Vernon and the framework guides give reasons, not data. The measured
  results are [Bagherzadeh] (186 Akka bugs), [TorresLopez] (23 bugs from the literature) and [Tasharofi] (15
  Scala programs). They show that these failures are real, not how often they happen elsewhere.
- **Not read.** Agha's 1986 book (only the publisher's page was reached). The books by Cesarini and Vinoski, by
  Vernon ("Reactive Messaging Patterns") and by Kuhn: contents pages only. The Enterprise Integration Patterns
  are the public pattern pages, not the book.
- **Living pages.** Framework pages change. Cloudflare's request rates and Stripe's 24 hours are their own
  numbers, on the day they were read.

## Sources

Read by the Claude run (**V**, **Q**), by Codex (**C**), or by both. `raw/codex.md` has Codex's notes on each of
its 76 sources.

| Name | Source | Read by |
| --- | --- | --- |
| [Hewitt73] | C. Hewitt, P. Bishop, R. Steiger, "A Universal Modular ACTOR Formalism for Artificial Intelligence", 1973. https://ijcai.org/Proceedings/73/Papers/027B.pdf | Codex |
| [Armstrong] | J. Armstrong, "Making reliable distributed systems in the presence of software errors", thesis, 2003. Appendix B reprints the 1996 Ericsson programming rules. https://erlang.org/download/armstrong_thesis_2003.pdf | both |
| [Armstrong-COP] | J. Armstrong, "Concurrency Oriented Programming in Erlang", 2003. https://guug.de/veranstaltungen/ffg2003/papers/ffg2003-armstrong.pdf | Codex |
| [OTP-sup] | Erlang/OTP, Supervision Principles. https://www.erlang.org/doc/system/sup_princ.html | both |
| [OTP-statem] | Erlang/OTP, gen_statem Behaviour. https://www.erlang.org/doc/system/statem.html | Codex |
| [OTP-ports] | Erlang/OTP, Ports and Port Drivers. https://www.erlang.org/doc/system/ports.html | both |
| [Hebert-zen] | F. Hebert, "The Zen of Erlang". https://ferd.ca/the-zen-of-erlang.html | both |
| [Hebert-guar] | F. Hebert, "It's About the Guarantees". https://ferd.ca/it-s-about-the-guarantees.html | both |
| [Hebert-anger] | F. Hebert, "Erlang in Anger", 2016. https://s3.us-east-2.amazonaws.com/ferd.erlang-in-anger/text.v1.1.0.pdf | Codex |
| [Vernon] | V. Vernon, "Effective Aggregate Design", parts I to III, 2011. https://www.dddcommunity.org/library/vernon_2011/ | both |
| [Helland-life] | P. Helland, "Life Beyond Distributed Transactions: an Apostate's Opinion", 2007. https://www.cidrdb.org/cidr2007/papers/cidr07p15.pdf | Codex |
| [Helland-idem] | P. Helland, "Idempotence Is Not a Medical Condition", 2012. https://queue.acm.org/detail.cfm?id=2187821 | Claude |
| [Quicksand] | P. Helland, D. Campbell, "Building on Quicksand", 2009. https://arxiv.org/pdf/0909.1788 | Codex |
| [Sagas] | H. Garcia-Molina, K. Salem, "Sagas", 1987. https://www.cs.princeton.edu/techreports/1987/070.pdf | both |
| [McCaffrey-sagas] | C. McCaffrey, "Distributed Sagas", 2017. https://speakerdeck.com/caitiem20/distributed-sagas-a-protocol-for-coordinating-microservices | Codex |
| [EIP] | G. Hohpe, B. Woolf, Enterprise Integration Patterns, the public pattern pages. https://www.enterpriseintegrationpatterns.com/patterns/messaging/ | Codex |
| [Pekko-IP] | Apache Pekko and Akka, Interaction Patterns. https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html | both |
| [Pekko-style] | Apache Pekko, Style Guide. https://pekko.apache.org/docs/pekko/current/typed/style-guide.html | both |
| [Akka-intro] | Akka, "How the Actor Model Meets the Needs of Modern, Distributed Systems". https://doc.akka.io/libraries/akka-core/current/typed/guide/actors-intro.html | Codex |
| [Akka-IoT] | Akka and Pekko guide, the IoT example, parts 3 to 5. https://doc.akka.io/docs/akka/current/typed/guide/tutorial_3.html | both |
| [Akka-persist] | Akka, Event Sourcing. https://doc.akka.io/docs/akka/current/typed/persistence.html | Claude |
| [Orleans-paper] | P. Bernstein et al., "Orleans: Distributed Virtual Actors for Programmability and Scalability", 2014. https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/Orleans-MSR-TR-2014-41.pdf | Codex |
| [Orleans-BP] | Microsoft Orleans, Best practices. https://learn.microsoft.com/en-us/dotnet/orleans/resources/best-practices | both |
| [Orleans-sched] | Microsoft Orleans, Request scheduling. https://learn.microsoft.com/en-us/dotnet/orleans/grains/request-scheduling | Codex |
| [Orleans-tx] | Microsoft Orleans, Transactions. https://learn.microsoft.com/en-us/dotnet/orleans/grains/transactions | Codex |
| [Orleans-ver] | Microsoft Orleans, Grain versioning. https://learn.microsoft.com/en-us/dotnet/orleans/grains/grain-versioning/grain-versioning | Codex |
| [Dapr] | Dapr, Actors overview. https://docs.dapr.io/developing-applications/building-blocks/actors/actors-overview/ | both |
| [Dapr-timers] | Dapr, Actor timers and reminders. https://docs.dapr.io/developing-applications/building-blocks/actors/actors-timers-reminders/ | Codex |
| [CF-rules] | Cloudflare, Rules of Durable Objects. https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/ | both |
| [Proto-sup] | Proto.Actor, Supervision. https://asynkron.se/protoactor/supervision/ | Codex |
| [Azure-entities] | Microsoft, Durable Task entities. https://learn.microsoft.com/en-us/azure/durable-task/common/durable-task-entities | Codex |
| [Azure-human] | Microsoft, Human interaction in Durable Task. https://learn.microsoft.com/en-us/azure/durable-task/common/durable-task-human-interaction | Codex |
| [Azure-CB] | Microsoft, Circuit Breaker pattern. https://learn.microsoft.com/en-us/azure/architecture/patterns/circuit-breaker | Codex |
| [Azure-comp] | Microsoft, Compensating Transaction pattern. https://learn.microsoft.com/en-us/azure/architecture/patterns/compensating-transaction | Codex |
| [Temporal] | Temporal, "Idempotency and Durable Execution". https://temporal.io/blog/idempotency-and-durable-execution | Claude |
| [Temporal-act] | Temporal, Activity Definition. https://docs.temporal.io/activity-definition | Codex |
| [Temporal-msg] | Temporal, Workflow message passing. https://docs.temporal.io/develop/typescript/workflows/message-passing | Codex |
| [Temporal-patterns] | Temporal, Design Patterns. https://docs.temporal.io/design-patterns | Codex |
| [Restate] | Restate, "Why we built Restate". https://restate.dev/blog/why-we-built-restate | Codex |
| [Restate-steps] | Restate, Durable steps. https://docs.restate.dev/develop/ts/durable-steps | Codex |
| [Outbox] | C. Richardson, Transactional outbox. https://microservices.io/patterns/data/transactional-outbox.html | Codex |
| [Saga-pattern] | C. Richardson, Saga. https://microservices.io/patterns/data/saga.html | Codex |
| [Stripe-idem] | Stripe, Idempotent requests. https://docs.stripe.com/api/idempotent_requests | Codex |
| [Stripe-webhooks] | Stripe, Receive events in your webhook endpoint. https://docs.stripe.com/webhooks | Codex |
| [Fowler-CQRS] | M. Fowler, CQRS. https://martinfowler.com/bliki/CQRS.html | Codex |
| [Escrow] | V. Balegas et al., "Extending Eventually Consistent Cloud Databases for Enforcing Numeric Invariants", 2015. https://arxiv.org/pdf/1503.09052 | Codex |
| [Bagherzadeh] | M. Bagherzadeh et al., "Actor Concurrency Bugs", OOPSLA 2020. https://mbagherz.bitbucket.io/lab-correct-software/papers/akka-actor-bugs.pdf | both |
| [TorresLopez] | C. Torres Lopez et al., "A Study of Concurrency Bugs and Advanced Development Support for Actor-based Programs". https://arxiv.org/pdf/1706.07372 | both |
| [Tasharofi] | S. Tasharofi, P. Dinges, R. Johnson, "Why Do Scala Developers Mix the Actor Model with Other Concurrency Models?", ECOOP 2013. https://ideals.illinois.edu/items/35044 | both |
| [P] | A. Desai et al., "P: Safe Asynchronous Event-Driven Programming", 2013. https://www.microsoft.com/en-us/research/publication/p-safe-asynchronous-event-driven-programming-2/ | both |
| [Rebeca] | M. Sirjani et al., "From Requirements to Verifiable Executable Models Using Rebeca", 2019. https://rebeca-lang.org/assets/papers/2019/From-Requirements-to-Verifiable-Executable-Models-Using-Rebeca.pdf | Codex |
| [TLA-AWS] | C. Newcombe et al., "Use of Formal Methods at Amazon Web Services". https://lamport.azurewebsites.net/tla/formal-methods-amazon.pdf | Codex |
| [Maty] | S. Fowler, R. Hu, "Speak Now: Safe Actor Programming with Multiparty Session Types". https://arxiv.org/pdf/2602.24054 | Claude |
| [Session-actors] | S. Fowler, "An Erlang Implementation of Multiparty Session Actors", 2016. https://arxiv.org/html/1608.03321 | Codex |
| [McCaffrey-Halo] | C. McCaffrey, "Clients Are Jerks: How Halo 4 DoSed the Services at Launch", 2015. https://caitiem20.wordpress.com/2015/06/23/clients-are-jerks-aka-how-halo-4-dosed-the-services-at-launch-how-we-survived/ | Codex |
| [Discord] | Discord, "Maxjourney: Pushing Discord's Limits with a Million+ Online Users in a Single Server", 2023. https://discord.com/blog/maxjourney-pushing-discords-limits-with-a-million-plus-online-users-in-a-single-server | Codex |
| [WhatsApp] | R. Reed, "Scaling to Millions of Simultaneous Connections", 2014. https://www.erlang-factory.com/static/upload/media/1394350183453526efsf2014whatsappscaling.pdf | Codex |
| [EventStorming] | A. Brandolini, EventStorming. https://www.eventstorming.com/ | Codex |
| [Isolation-trap] | "The Isolation Trap", an essay critical of the model, with a rebuttal it quotes. A blog, not a primary source. https://causality.blog/essays/the-isolation-trap/ | Claude |
