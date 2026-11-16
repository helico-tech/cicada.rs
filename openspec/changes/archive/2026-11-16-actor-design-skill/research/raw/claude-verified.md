# Claude deep research: raw result

## Stats
{
 "angles": 5,
 "sourcesFetched": 24,
 "claimsExtracted": 115,
 "claimsVerified": 25,
 "confirmed": 25,
 "killed": 0,
 "unverified": 0,
 "afterSynthesis": 16,
 "urlDupes": 2,
 "budgetDropped": 4,
 "agentCalls": 106
}

## Summary
The verified evidence supports a clear core method. Find actors from the problem (one per truly concurrent activity or unit of coordination, sized to a true invariant), give each invariant one owning actor, and treat everything that spans actors as an eventually consistent exchange of idempotent messages, using a saga (forward retry or semantic compensation) when several steps must all happen or be undone. Design the protocol first: request-reply is two messages with a reply address and a correlation id, nothing may depend on the last message arriving or on a timeout meaning 'it did not happen', and each actor keeps foreign replies out of its public vocabulary. Design failure as responsibility: foreseen outcomes are replies in normal code, unforeseen faults crash the worker, one owner restarts it, restart scope follows the dependencies, repeated faults escalate, and the part that must be correct (the error kernel) is identified first. All 25 claims were checked against seven primary sources with 3-0 votes, but most are one school's opinion (Erlang/OTP, Vernon's DDD, Pekko/Akka, Cloudflare), and applying aggregates, sagas and service dialogs to actors is inference. The set holds no verified claim on time (E), the outside world (G), most anti-patterns (H) or method and verification (I), so those parts of the skill cannot yet be written from verified evidence.

## findings (16)

### findings 1

- **claim**: Finding the actors: start from the problem, not the code. Make one actor per truly concurrent real-world activity (Armstrong). For data-centred systems that means one per logical unit that needs coordination (Cloudflare), which lines up with a DDD aggregate: a consistency boundary around true invariants (Vernon). Then name the channels between actors and write down every message on each channel. Whatever a function call can do stays a function or a value. Status: three primary sources agree on the shape; each is one school's opinion, and 'aggregate = actor' is inference.
- **confidence**: high
- **sources**: ["https://erlang.org/download/armstrong_thesis_2003.pdf", "https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/", "https://www.dddcommunity.org/library/vernon_2011/", "https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_1.pdf"]
- **evidence**: Armstrong thesis sec. 2.4.1 (p. 21) gives three steps: identify the truly concurrent activities, identify the message channels between them, write down all messages that can flow on each channel. Then: 'Each real world concurrent activity should be mapped onto exactly one concurrent process ... It is extremely important that the mapping is exactly 1:1.' A mapping that is not 1:1 'will quickly degenerate' and leads to 'interference errors'. Appendix B rule 5.2 (p. 228): 'don't use processes and message passing when a function call can be used instead'. Cloudflare: 'The most important design decision is choosing what each Durable Object represents. Create one Durable Object per logical unit that needs coordination: a chat room, a game session, a document, a user's data, or a tenant's workspace.' The rule before it on that page is whether to use an actor at all: for stateful coordination, not stateless request handling. Vernon Part I: 'aggregate is synonymous with transactional consistency boundary'. Limits: Armstrong's rule is argued, not measured, and scoped to programs that model or interact with the world. Appendix B is a 1996 Ericsson guideline by Eriksson, Williams and Armstrong, and the thesis does not itself link rule 5.2 to the 1:1 rule. Vernon never mentions actors. Calling Cloudflare's 'atom of coordination' a consistency boundary is inference; the page does list strong consistency as a reason to use an object. Cloudflare says 'most important', not 'first'.
- **vote**: 3-0 on each of claims 0, 7, 23

### findings 2

- **claim**: Granularity: size an actor to its invariant, meaning the root entity plus only the data that must stay consistent together. Too coarse shows as unrelated user actions getting in each other's way, and poor performance and scalability. Too fine shows as a true invariant that no single actor protects. A second limit is load: if the expected request rate is more than one actor can serve, split by a natural key. Status: well supported by three primary sources; the numbers are platform-specific.
- **confidence**: high
- **sources**: ["https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_1.pdf", "https://www.dddcommunity.org/library/vernon_2011/", "https://erlang.org/download/armstrong_thesis_2003.pdf", "https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/"]
- **evidence**: Vernon Part I: 'We could fall into the trap of designing for compositional convenience and make them too large. At the other end of the spectrum we could strip all aggregates bare, and as a result fail to protect true invariants.' The large aggregate 'was deficient from the start because the false invariants and a desire for compositional convenience drove the design, to the detriment of transactional success, performance, and scalability.' Symptom: 'Nothing about planning a new backlog item should logically interfere with scheduling a new release!' Right size: 'limit the aggregate to just the root entity and a minimal number of attributes and/or value-typed properties. The correct minimum is the ones necessary, and no more', where necessary means 'those that must be consistent with others, even if domain experts don't specify them as rules'. Armstrong: choose a granularity, 'one process per user and not one process for every atom in the user's body'. Cloudflare: 'When modeling your "atom," factor in the expected request rate. If your use case exceeds these limits, shard your workload across multiple Durable Objects', with about 500-1,000 requests per second per object for simple operations and about 200-500 for complex operations with storage writes. Limits: the '70% of aggregates were a single root entity' figure is one project's second-hand report and Vernon says it does not generalise. Vernon allows several entities in one aggregate when a true invariant needs it, and Part III asks whether small aggregates can be overdone. The Cloudflare numbers are one platform's rough figures, to be used as a condition ('if one actor cannot carry the load'), not as a constant.
- **vote**: 3-0 on each of claims 0, 8, 23, 24

### findings 3

- **claim**: One invariant, one owner. A command changes one actor. If a single step must change several actors together, the boundaries are probably wrong or an invariant was missed. A rule that spans actors is met by eventual consistency: the first actor publishes an event, each subscriber updates one other actor in its own step, delivery is retried, and on complete failure the design compensates or at least reports the failure for someone to act on. Tie-breaker: if making the data consistent is the job of the user running the use case, try to keep it in one boundary; if it is another user's or the system's job, make it eventual. Status: Vernon's rule of thumb for aggregates, widely adopted in DDD; applying it to actors is inference.
- **confidence**: medium
- **sources**: ["https://www.dddcommunity.org/library/vernon_2011/", "https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_1.pdf", "https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_2.pdf"]
- **evidence**: Vernon Part I: 'a properly designed bounded context modifies only one aggregate instance per transaction in all cases', which he calls 'a rule of thumb' that 'should be the goal in most cases'. Part II: 'If you are modifying multiple instances in a single transaction, it may be a strong indication that your consistency boundaries are wrong.' Part I: keeping several instances consistent 'may be telling you that your team has missed an invariant'. Part II, 'Use Eventual Consistency Outside the Boundary': 'if executing a command on one aggregate instance requires that additional business rules execute on one or more other aggregates, use eventual consistency'; 'Each of the subscribers executes in a separate transaction'; 'This retry process can continue until consistency is achieved, or until a retry limit is reached. If complete failure occurs it may be necessary to compensate, or at a minimum to report the failure for pending intervention.' Tie-breaker ('Ask Whose Job It Is', which Vernon reports from a talk with Eric Evans): 'If it is, try to make it transactionally consistent, but only by adhering to the other rules of aggregate. If it is another user's job, or the job of the system, allow it to be eventually consistent.' It 'exposes the real system invariants'; domain experts often accept delays of 'seconds, minutes, hours, or even days'. Limits: Vernon hedges ('may be', 'try'); the tie-breaker is not a licence to grow the actor, and he says it 'may not always lead to the final answer'. He lists four reasons to break the one-per-transaction rule: user interface convenience, lack of technical mechanisms (messaging, timers or background threads), global transactions, query performance. The Evans attribution is second-hand. Review use: 'an invariant split over several actors' is the anti-pattern this finding names.
- **vote**: 3-0 on each of claims 7, 10, 11

### findings 4

- **claim**: Actors refer to each other by identity only, never by holding a direct reference to the other's state. This keeps each one small, removes the temptation to change two in one step, and lets state be moved and distributed. The cost: a view over several actors needs several lookups. When that hurts, serve reads from a separate read model (CQRS) instead of widening an actor. Status: Vernon's rule for aggregates, worded as 'Prefer' with exceptions; its use for actors is inference.
- **confidence**: medium
- **sources**: ["https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_2.pdf", "https://www.dddcommunity.org/library/vernon_2011/"]
- **evidence**: Vernon Part II, 'Rule: Reference Other Aggregates By Identity': 'Prefer references to external aggregates only by their globally unique identity, not by holding a direct object reference (or pointer).' 'If you don't hold any reference, you can't modify another aggregate.' 'Since aggregates don't use direct references to other aggregates, but reference by identity, their persistent state can be moved around to reach large scale', which he ties to Helland ('What we call aggregate he calls entity'). Cost: 'You may have to use multiple repositories in a single use case to populate views. If query overhead causes performance issues, it may be worth considering the use of CQRS.' Limits: CQRS is conditional and one of two options (the other is mixing identity and direct references). Vernon notes Evans's DDD allows an aggregate to hold references to the roots of others, and Part III shows a case for breaking the rule.
- **vote**: 3-0 (claim 9)

### findings 5

- **claim**: A business process over several actors is a saga: a sequence of local steps where either all steps complete, or the completed ones are compensated in reverse order (T1..Tj, then Cj..C1). Isolation is given up, so others can see half-finished results; the design must therefore keep in-flight state explicit and visible (a 'pending' or 'in transit' record) and not in the process's private variables. Put step boundaries at natural divisions, often one per real-world action. A compensation is a new business action (cancel the reservation), not a restore of old state, and some effects can only be countered, not undone. If the process has no user-initiated abort and every step eventually succeeds when retried, forward recovery alone is enough and no compensations are needed. Status: well established as a definition; the paper covers one central DBMS, so its use for actors, messaging, and orchestration versus choreography is later inference.
- **confidence**: high
- **sources**: ["https://www.cs.princeton.edu/research/techreps/598", "https://www.cs.princeton.edu/techreports/1987/070.pdf"]
- **evidence**: Garcia-Molina and Salem, 'Sagas' (Princeton TR-070-87 and SIGMOD 1987). Definition: 'A LLT is a saga if it can be written as a sequence of transactions that can be interleaved with other transactions.' Guarantee: either T1..Tn or 'T1, T2, ... Tj, Cj, ... C2, C1'. Isolation: 'sagas may view the partial results of other sagas'. Compensation: it 'undoes, from a semantic point of view, any of the actions performed by Ti, but does not necessarily return the database to the state that existed when the execution of Ti began', because other work may have run in between (the seat-count example). Hard cases: 'if a transaction fires a missile, it may not be possible to undo this action'; 'to compensate for the letter, send a second letter explaining the problem. To compensate for the check, send a stop-payment message to the bank.' Recovery: 'compensate for the executed transactions, backward recovery, or execute the missing transactions, forward recovery'; 'Without these aborts, pure forward recovery is feasible and compensation is never needed', with the footnote 'we must also assume that every sub-transaction in the saga will eventually succeed if it is retried enough times'. Step design (sec. 9): 'search for natural divisions of the work', each real-world action 'is a candidate'; lay data out as 'loosely-coupled components (with few and simple inter-component consistency constraints)'; store temporary data in the database (a relation for 'funds in transit') and minimise 'data passed from one sub-transaction to the next via local storage'. Limits: the paper hedges ('may not be possible', 'in many cases ... a candidate'). Forward recovery also needs a save-point at the start of every step, and a step with a bug leaves the saga stuck until someone intervenes. Later vendor guidance (AWS, Azure), cited by the verifier, adds that retried steps must be idempotent in a distributed setting. The paper has one central saga executor and a log; it does not discuss actors or orchestration versus choreography.
- **vote**: 3-0 on each of claims 12, 13, 14, 15

### findings 6

- **claim**: If a message is marked consumed only after it is processed (at-least-once delivery, which Helland calls the most common case), then every handler must be idempotent. Idempotence is designed into the message: name the specific piece of work with an identifier, and have the receiver apply it only if that identifier was not processed before. Extra log lines or resource use on a repeat do not break idempotence. Status: well established; Helland wrote about dialogs between two services, so applying it to actors is inference, and the terms 'business key' and 'idempotency key' are not his.
- **confidence**: high
- **sources**: ["https://queue.acm.org/detail.cfm?id=2187821", "https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_2.pdf"]
- **evidence**: Helland, 'Idempotence Is Not a Medical Condition' (ACM Queue 10(4), 2012): 'The message is consumed after processing. This is the most common case. The application must be designed so that each and every message is idempotent in its processing. There is a failure window in which the work is successfully applied to the database, but a failure prevents the messaging system from knowing the message was consumed. The messaging system will re-drive the delivery of the message.' Summary rule: 'Every message may be retried and, hence, must be idempotent.' Transport promise: 'delivered zero or more times'. Design of the message: 'Withdrawing $1 billion is not idempotent' but 'Processing withdrawal XYZ for $1 billion if not already processed is idempotent.' Definition: 'Acting as if used only once, even if used multiple times'; logging, monitoring and resource side effects 'are not relevant to the semantics'. Vernon's cross-aggregate rule assumes the same redelivery ('The message will be redelivered, a new transaction started'). Limits: Helland notes that shared plumbing can remove duplicates after the first reply of a dialog, so the rule is not fully unconditional. He excludes pub-sub. He does not say how the receiver records 'already processed' (for example in the same atomic step as the state change) or how long to keep the record; that needs another source. The live URL returned HTTP 403; the text was read from Internet Archive snapshots.
- **vote**: 3-0 on each of claims 16, 17 (10 as corroboration)

### findings 7

- **claim**: No design may depend on a sender knowing that its last message arrived, and a timeout on a request does not mean the work did not happen. The second-to-last message of a conversation can be confirmed; the last one is best effort. After an asker times out, the receiver may still finish the work or even start it later. So 'outcome unknown' must be a designed case. Status: well established (it is the Two Generals result in protocol form); the remedy 'retry with the same identifier or query the state' is inference from combining the two sources.
- **confidence**: high
- **sources**: ["https://queue.acm.org/detail.cfm?id=2187821", "https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html"]
- **evidence**: Helland: 'In any interaction, the last message from one application service to another cannot be guaranteed. The only way to know it was received is to send a message saying it was. That means it's no longer the last message.' and 'The application must not really care if that last message is received because you can't really tell if it is.' Pekko/Akka docs, problems of ask: 'When ask times out, the receiving actor does not know and may still process it to completion, or even start processing it after the fact'. Appendix B rule 5.11 of Armstrong's thesis adds: handle the reply that arrives after a timeout. Limits: with durable retries the last message can still arrive in the end; what is impossible is for the sender to know it without one more message. A transport acknowledgement only moves the problem: the ack becomes the best-effort last message, and its loss causes a resend the receiver must treat idempotently. One of three verifiers could not load Helland's article and relied on search-surfaced article text plus a secondary summary for this passage.
- **vote**: 3-0 on each of claims 18, 21

### findings 8

- **claim**: Request-reply is two one-way messages. The request carries the reply address as a field and a correlation value (a request id echoed in the reply). The reply has its own tag, different from the request. Without a correlation value, a reply can only be tied to its context by adding a separate actor (ask, or a per-session child). Plain request-reply also makes it hard to see that a request was never delivered or processed. Status: two primary sources agree; stated for Akka/Pekko Typed, since Classic Akka passes the sender implicitly.
- **confidence**: high
- **sources**: ["https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html", "https://erlang.org/download/armstrong_thesis_2003.pdf"]
- **evidence**: Pekko docs, Request-Response: 'In Pekko, the recipient of responses has to be encoded as a field in the message itself, which the recipient can then use to send (tell) a response back.' Problems: 'Unless the protocol already includes a way to provide context, for example a request id that is also sent in the response, it is not possible to tie an interaction to some specific context without introducing a new, separate, actor (see ask or per session child actor)' and 'It is hard to detect that a message request was not delivered or processed'. The current Akka docs carry the same text. Armstrong thesis Appendix B rule 5.7 (p. 230): 'All messages should be tagged' and the reply tagged with a new atom. Limits: in Classic Akka/Pekko the reply address travels implicitly through sender(). The 'separate actor' behind ask is a hidden temporary reference made by the library, not one the designer writes. That this matches the general actor model (customer-passing style) is the claimant's inference, confirmed only from course notes, not from Agha 1986.
- **vote**: 3-0 on each of claims 19, 3

### findings 9

- **claim**: Use ask (request-reply with a timeout) for a single-response query, for 'I must know this was processed before I go on', for resending, or to limit outstanding requests. Its costs: one reply only, the timeout does not cancel the work, and timeouts are hard to choose when one ask triggers further asks (ask chains). When one request needs several answers (aggregation, scatter-gather, retry and acknowledgement), hand the conversation to a short-lived per-session child actor that holds the conversation state, replies and stops itself. Its protocol is private to the parent. Costs of the child: it must be ended on every path or it leaks, it normally needs a timeout, and it runs concurrently with its parent and siblings. Status: one framework family's documented advice (Pekko/Akka), worded with hedges.
- **confidence**: medium
- **sources**: ["https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html"]
- **evidence**: Ask, Problems: 'There can only be a single response to one ask', 'When ask times out, the receiving actor does not know and may still process it to completion, or even start processing it after the fact', 'Finding a good value for the timeout, especially when ask triggers chained asks in the receiving actor.' Ask, Useful when: 'Single response queries', 'An actor needs to know that the message was processed before continuing', 'To allow an actor to resend if a timely response is not produced', 'To keep track of outstanding requests and not overwhelm a recipient with messages (backpressure)'; a fifth bullet covers attaching context when the protocol has no request id. Per-session child: 'In some cases a complete response to a request can only be created and sent back after collecting multiple answers from other actors. For these kinds of interaction it can be good to delegate the work to a per session child actor.' 'the protocol of the session actor is not a public API but rather an implementation detail of the parent actor.' Problems: 'Children have life cycles that must be managed to not create a resource leak, it can be easy to miss a scenario where the session actor is not stopped' and 'It increases complexity, since each such child can execute concurrently with other children and the parent'. 'if all you need is a single response with a timeout it is better to use ask.' Limits: the docs say 'it can be good' and 'would likely want' a timeout; 'scatter-gather' is the report's label, not the docs' word. The Akka docs carry the same text. The three ask problems are for ask between two actors; ask from outside an actor has a different list.
- **vote**: 3-0 on each of claims 21, 22

### findings 10

- **claim**: Each actor owns its message vocabulary. An actor should not take another actor's reply types into its public protocol; it translates foreign replies into its own message types (a message adapter) or keeps them in a private internal type. The older Erlang rule points the same way from another side: hide the message protocol behind interface functions. Status: hedged guidance from one framework family (Pekko/Akka Typed), driven by typed protocols; the Erlang rule comes from the 1996 Ericsson conventions.
- **confidence**: medium
- **sources**: ["https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html", "https://erlang.org/download/armstrong_thesis_2003.pdf"]
- **evidence**: Pekko docs: 'Most often the sending actor does not, and should not, support receiving the response messages of another actor.' and 'Actors seldom have a response message from another actor as a part of their protocol as it can be considered as polluting that protocol with a message from another actor's message (see adapted response)'. The Pekko style guide says response wrappers are internal and should be private. Armstrong thesis Appendix B rule 5.10 (pp. 232-233): 'Encapsulate message passing into interface functions' because 'The message protocol is internal information and should be hidden to other modules'. Limits: the wording is hedged ('Most often', 'seldom', 'can be considered'). The adapter is one mechanism: with Scala 3 union types the foreign replies sit in a private internal type with no adapter, so the invariant is 'keep foreign replies out of the public protocol'. The page documents one exception: when replying to a sharded actor by entity id, the response must be in that actor's protocol. 'Polluting' is Pekko's word; Akka's current page drops it. A shared protocol in a separate object is allowed when several actors share it. That untyped Erlang designs do not state this rule was not checked against Erlang sources.
- **vote**: 3-0 on each of claims 20, 3

### findings 11

- **claim**: Design the protocol first, as a state machine: for each protocol state and incoming message, the allowed replies and the next state. This makes the conversation reviewable and checkable, and a violation points at the faulty side. Supporting rules: tag every message, handle unknown messages, and consider stating time bounds in the protocol. Status: Armstrong's own proposal; the run-time 'contract checker' (UBF) is not mainstream practice.
- **confidence**: medium
- **sources**: ["https://erlang.org/download/armstrong_thesis_2003.pdf"]
- **evidence**: Armstrong thesis ch. 9 (pp. 195-199): the protocol between two components 'can be described in terms of a non-deterministic finite state machine'; a 'protocol checking' program 'can be placed between any pair of processes'; 'Of these two methods I prefer the use of a contract checker' over a type system on the language; 'If we formally describe the protocol to be obeyed on a communication channel between two black boxes then we can use this as a means for detecting and identifying errors, we can also say precisely which component has failed.' Time bounds: 'we might add to our protocol specification language a notion of time' ('within T1'). Appendix B: rule 5.7 tag all messages, rule 5.8 'Every server should have an Other alternative' for unknown messages. Limits: his stated reasons for preferring the checker are that components are black boxes and that an external checker does not interfere with building the component; fault identification is a separately stated benefit. The time bound is a hypothetical extension. The checker needs the server to return its protocol state with each reply, and is described for client-server request-reply only. Rule 5.8's stated purpose is to stop message queues filling up. Mailing-list posts describe UBF as inactive; 'not mainstream' is the claimant's assessment, since no source measures adoption.
- **vote**: 3-0 (claim 3)

### findings 12

- **claim**: Separate business failures from faults, using the specification as the line. A condition the programmer foresaw and can correct (Armstrong's example: opening a file that does not exist) is not an error: handle it in normal code. An error is what the programmer does not know how to handle: the actor should crash, and a different actor deals with it. So workers do the work and supervisors handle failure. Status: the primary source for 'let it crash'; well established within Erlang/OTP, with a known limit: do not crash away vital data.
- **confidence**: high
- **sources**: ["https://erlang.org/download/armstrong_thesis_2003.pdf", "https://www.erlang.org/doc/system/sup_princ.html"]
- **evidence**: Armstrong thesis sec. 4.3-4.4 (pp. 104-108): 'The philosophy is let some other process fix the error, but what does this mean for their code? The answer is let it crash. By this I mean that in the event of an error, then the program should just crash.' 'The processes that are supposed to do things (the workers) do not have to worry about error handling.' 'errors occur when the programmer doesn't know what to do'; a foreseen exception the programmer can correct 'is not an error'. Sec. 5.3 (p. 124) defines an error as a deviation from 'the behaviour that the specification says the system should have'. The OTP docs give the matching supervisor role: 'A supervisor is responsible for starting, stopping, and monitoring its child processes.' Limits: Armstrong adds that specifications are often incomplete and 'It is the programmer who decides if an exception corresponds to an error'. Appendix B rule 3.13 limits 'do not program defensively': check data once, where it enters the system. Vinoski's QCon 2011 talk 'Let It Crash... Except When You Shouldn't' (cited by the verifier, not checked as a claim) warns that crashing is no excuse for losing vital data. That business failures should be sent as reply messages or data is the report's extension; the thesis says only 'handled in normal code'.
- **vote**: 3-0 on each of claims 1, 4

### findings 13

- **claim**: The first step of failure design is to decide which part of the system must be correct and which part may fail. The must-be-correct part is the error kernel; it often holds the state that matters. Give each actor one role where possible: supervisor, worker that may have errors, or trusted worker that may not. When the system cannot do what it wants, it should try something simpler, done by simpler software. Status: primary source (the 1996 Ericsson rules reprinted in Armstrong's thesis); well established in Erlang/OTP practice; 'valuable state' is a later reading of the source's words.
- **confidence**: medium
- **sources**: ["https://erlang.org/download/armstrong_thesis_2003.pdf"]
- **evidence**: Appendix B rule 4.2 (p. 228): 'The first part of the system design must be to identify that part of the system which must be correct; we call this the error kernel. Often the error kernel has some kind of real-time memory resident data base which stores the state of the hardware.' Rule 5.5 (p. 229): 'Each process should only have one role', listing supervisor, worker ('can have errors') and trusted worker ('not allowed to have errors'). Chapter 5.1 (p. 116): 'if we cannot do what we want to do, then try to do something simpler', with simpler tasks performed by simpler software. Limits: the source says 'the state of the hardware'; generalising to 'valuable state' matches the later Akka error-kernel pattern but is a gloss. The thesis does not itself link the error kernel to the 'try something simpler' strategy; that link is the claimant's. Rule 5.5 is hedged ('as far as possible') and is mainly about client versus server. The 1996 document calls itself preliminary. 'Risky work at the leaves' is not in this source.
- **vote**: 3-0 (claim 2)

### findings 14

- **claim**: Supervision is a design of responsibility, not a retry mechanism. Each supervised actor has one owner that starts, stops and restarts it. The restart strategy decides how far one failure spreads: only the failed child, all children, or the failed child plus those started after it; so the start order of children encodes dependencies. Restarts are bounded in rate: a child that fails too often takes its supervisor down, and the fault becomes the parent's problem. Status: the mechanics are well established (primary OTP docs); 'the strategy should mirror the dependencies' is inference, corroborated by Hebert; the single-owner rule holds for supervision-tree systems only.
- **confidence**: high
- **sources**: ["https://www.erlang.org/doc/system/sup_princ.html", "https://www.erlang.org/doc/apps/stdlib/supervisor.html", "https://learnyousomeerlang.com/supervisors"]
- **evidence**: OTP Supervision Principles: 'A supervisor is responsible for starting, stopping, and monitoring its child processes. The basic idea of a supervisor is that it is to keep its child processes alive by restarting them when necessary.' rest_for_one: 'If a child process terminates, the child processes after the terminated process in start order are terminated. Subsequently, the terminated child process and the remaining child processes are restarted.' Children 'are started in the order specified by this list, and are terminated in the reverse order'. Escalation: 'If more than MaxR number of restarts occur in the last MaxT seconds, the supervisor terminates all the child processes and then itself. ... When the supervisor terminates, then the next higher-level supervisor takes some action. It either restarts the terminated supervisor or terminates itself.' Defaults are 1 restart in 5 seconds. The page gives no 'when to use' rule; Hebert's Learn You Some Erlang does (one_for_one for independent children, one_for_all when they depend heavily on each other, rest_for_one for a chain of dependencies). Limits: 'exactly one owner' is a paraphrase that follows from the tree structure. A temporary child is never restarted and a transient child only after an abnormal exit; a transient child supervisor that hits its limit exits with 'shutdown' and is not restarted. 'Bounded' means bounded in rate: a fault that repeats more slowly than the limit is restarted forever. Total restarts multiply across levels, so the same intensity should not be used at every level. Virtual-actor systems (Orleans, Dapr, Durable Objects) have no supervising owner; that statement is the verifier's and was not checked against those systems' docs.
- **vote**: 3-0 on each of claims 4, 5, 6

### findings 15

- **claim**: Anti-pattern, singleton bottleneck: a single global actor that all traffic passes through (a global rate limiter or global counter under one fixed id). Symptom: throughput is capped by one actor and it becomes a chokepoint as traffic grows. Fix: find the natural coordination boundary (per user, per room, per document) and make one actor for each. A singleton that handles only low-rate control work, with the data path going around it, is accepted. Status: one vendor's guidance (Cloudflare); it matches the granularity rules above.
- **confidence**: medium
- **sources**: ["https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/"]
- **evidence**: Cloudflare, 'Anti-patterns to avoid: Do not use a single Durable Object as a global singleton': 'A single Durable Object handling all traffic becomes a bottleneck. While async operations allow request interleaving, all synchronous JavaScript execution is single-threaded, and storage operations provide serialization guarantees that limit throughput. A common mistake is using a Durable Object for global rate limiting or global counters. This funnels all traffic through a single instance.' Fix: 'identify natural coordination boundaries in your application (per user, per room, per document) and create separate Durable Objects for each.' Sizing: about 500-1,000 requests per second per object for simple operations, lower for heavier work, with 'Required DOs = (Total requests/second) / (Requests per DO capacity)'. Limits: the numbers are approximate and Cloudflare-specific. Cloudflare's own control-plane and data-plane reference architecture accepts one object per resource type for low-rate control work, so the anti-pattern is 'all traffic through one actor', not any singleton. The page was last updated on 5 November 2026 and may change.
- **vote**: 3-0 (claim 24)

### findings 16

- **claim**: Where the verified sources differ, as input for the comparison table: (1) Lifecycle owner: OTP and Akka give each actor one supervising parent; virtual-actor systems (Orleans, Dapr, Durable Objects) have none. (2) What the public API is: Akka/Pekko Typed makes the message types the public protocol; the Erlang rules hide messages behind interface functions. (3) How a protocol is checked: Akka/Pekko uses static types; Armstrong prefers a run-time contract checker over a type system. (4) Reply address: an explicit field in Typed; an implicit sender in Classic Akka. (5) Defensive code: Armstrong says do not program defensively, yet the same rule set says check data once at the system boundary, and Vinoski warns against crashing away vital data. (6) Sizing lens: Vernon sizes by invariant, Cloudflare by coordination unit bounded by request rate, Armstrong by real-world concurrency. (7) Transaction scope: Vernon's one-aggregate-per-step is a rule of thumb with four stated exceptions. Status: mostly inference from the verified claims and verifier notes; items 1 and 5 rest on sources that were not themselves verified as claims.
- **confidence**: low
- **sources**: ["https://www.erlang.org/doc/system/sup_princ.html", "https://erlang.org/download/armstrong_thesis_2003.pdf", "https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html", "https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/", "https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_2.pdf"]
- **evidence**: Each item is a contrast drawn between verified texts, not a statement found in one source. (2) Pekko: session and wrapper protocols are private while the command type is the public behaviour type; Armstrong Appendix B rule 5.10: 'The message protocol is internal information and should be hidden to other modules'. (3) Armstrong p. 197: 'We can impose a type system on our programming language, or we can impose a contract checking mechanism between any two components ... Of these two methods I prefer the use of a contract checker.' (4) Pekko's from-classic page, read by a verifier: 'There is no sender() in Typed'. (5) Thesis p. 104 slogans 'Let it crash' and 'Do not program defensively' versus Appendix B rule 3.13 and Vinoski's QCon 2011 talk, both cited only in verifier notes. (6) See the granularity finding. (7) Vernon Part II, 'Reasons To Break the Rules'. Item 1 is the verifier's statement about virtual-actor systems; no Orleans, Dapr or Durable Objects text on supervision was checked.
- **vote**: derived from claims 0-24; not voted on as a separate claim


## caveats (3575)

Coverage is the main limit. The 25 claims come from seven sources: Armstrong's thesis, the OTP supervision docs, Vernon's 'Effective Aggregate Design', Garcia-Molina and Salem's 'Sagas', Helland's 'Idempotence Is Not a Medical Condition', the Pekko/Akka interaction-patterns page and Cloudflare's Durable Objects rules. They cover parts of A (decomposition, granularity), C (protocol), D (consistency, sagas) and F (failure). The set holds no verified claim for E (time), G (the outside world, asked for in extra depth), I (method, worked examples, verification), or most of B (state machines, bounded state). For H, only these anti-patterns have verified support: too coarse, too fine, an invariant split over actors, singleton bottleneck, ask chains, and an actor where a function would do. There is no verified support here for anaemic CRUD actors, a shared database as side channel, mutable state leaking in messages, cyclic dependencies, hidden reliance on order or timing, unbounded state, or chatty protocols. The set also holds no claim from Hewitt or Agha, the Orleans papers and docs, Dapr, Hebert's essays, Cesarini and Vinoski, Helland's 'Life Beyond Distributed Transactions', Hohpe and Woolf, Kuhn, Vernon's actor book, the empirical bug studies, P or Rebeca, or critiques of the model. A pattern catalogue, an outside-world decision guide and a full frameworks table cannot be built from this evidence alone.

Inference. Three transfers carry much of the guidance and none is stated in its source: aggregate to actor (Vernon never mentions actors), saga to actor process (the paper is about one central DBMS), and service dialog to actor conversation (Helland). Several rules are argued, not measured (Armstrong), or are hedged rules of thumb with stated exceptions (Vernon, Pekko).

Confidence labels. Every claim passed 3-0, so the vote does not separate them. High means more than one primary source points the same way, or the finding is the canonical definition from the paper that introduced it. Medium means one primary source, checked verbatim, that is one school's opinion. Low means synthesis from verifier notes.

Wording the verifiers corrected, to keep in the report: Cloudflare says 'most important' design decision, not 'first'. Helland's 'every message must be idempotent' is qualified by his note that shared plumbing can remove duplicates. Vernon says it 'may be necessary' to compensate, with reporting as the minimum. 'Exactly one owner' is a paraphrase of the OTP docs. Restarts are bounded in rate, not in count, and a transient child is not restarted after a 'shutdown' exit. The last message can still arrive; the sender just cannot know it did. Armstrong's stated reasons for the contract checker are black-box components and non-interference with their construction; fault identification is a separate benefit.

Citation precision. The Vernon quotes are in the Part I and Part II PDFs, not on the cited landing page, and one quoted passage joins text from both parts. The Sagas quotes are in the TR-070-87 PDF linked from the cited page. Helland's article returned HTTP 403 live and was read from Internet Archive snapshots; one verifier of the 'last message' claim could not load it at all and relied on search-surfaced text and a secondary summary.

Time sensitivity. The Pekko page (version 2.0.0) and the Cloudflare page (updated 5 November 2026) are living documents; the Cloudflare request-rate figures are the most likely to change and are platform-specific. The other sources are historical texts or long-stable documentation.

## openQuestions (4)

- The outside world (section G): what do primary sources say about a side effect that happened before the actor recorded it (idempotency keys towards external systems, transactional outbox, reconciliation), about one actor per external resource (Erlang ports and their owner process, device actors in the Akka IoT guide), and about rate limits and circuit breakers as actor state? No claim in this set covers it.
- Do virtual-actor systems (Orleans, Dapr, Durable Objects) give different design advice on ownership, lifecycle and failure than supervision-tree systems (OTP, Akka)? The contrast in this report is a verifier's remark, not checked against the Orleans papers, Orleans best practices or Dapr guidance.
- Is 'one aggregate = one actor' stated and qualified by a primary source (Vernon's 'Reactive Messaging Patterns', Akka or Orleans documentation), and when does it fail, for example for rules that span many entities or for read-heavy views?
- Time, method and proof (sections E and I): how should timeouts, reminders and 'it did not happen' detection be modelled as messages; and which design mistakes do the empirical studies (Tasharofi 2013, Bagherzadeh 2020, Torres Lopez) and the verification tools (TLA+, P, Rebeca, session types) show to matter in practice, so that the argued rules here can be ranked by measured evidence?

## refuted (0)


## unverified (0)


## sources (24)

### sources 1

- **url**: https://erlang.org/download/armstrong_thesis_2003.pdf
- **quality**: primary
- **angle**: Foundations: decomposition and failure design (Erlang/OTP primary sources)
- **claimCount**: 5

### sources 2

- **url**: https://ferd.ca/the-zen-of-erlang.html
- **quality**: blog
- **angle**: Foundations: decomposition and failure design (Erlang/OTP primary sources)
- **claimCount**: 5

### sources 3

- **url**: https://ferd.ca/it-s-about-the-guarantees.html
- **quality**: blog
- **angle**: Foundations: decomposition and failure design (Erlang/OTP primary sources)
- **claimCount**: 5

### sources 4

- **url**: https://www.erlang.org/doc/system/sup_princ.html
- **quality**: primary
- **angle**: Foundations: decomposition and failure design (Erlang/OTP primary sources)
- **claimCount**: 5

### sources 5

- **url**: https://queue.acm.org/detail.cfm?id=3025012
- **quality**: unreliable
- **angle**: Consistency and coordination: aggregates, entities, sagas
- **claimCount**: 0

### sources 6

- **url**: https://www.dddcommunity.org/library/vernon_2011/
- **quality**: primary
- **angle**: Consistency and coordination: aggregates, entities, sagas
- **claimCount**: 5

### sources 7

- **url**: https://www.cs.princeton.edu/research/techreps/598
- **quality**: primary
- **angle**: Consistency and coordination: aggregates, entities, sagas
- **claimCount**: 5

### sources 8

- **url**: https://queue.acm.org/detail.cfm?id=2187821
- **quality**: primary
- **angle**: Consistency and coordination: aggregates, entities, sagas
- **claimCount**: 5

### sources 9

- **url**: https://pekko.apache.org/docs/pekko/current/typed/interaction-patterns.html
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 10

- **url**: https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 11

- **url**: https://learn.microsoft.com/en-us/dotnet/orleans/resources/best-practices
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 12

- **url**: https://pekko.apache.org/docs/pekko/current/typed/guide/tutorial_3.html
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 13

- **url**: https://pekko.apache.org/docs/pekko/current/typed/style-guide.html
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 14

- **url**: https://docs.dapr.io/developing-applications/building-blocks/actors/actors-overview/
- **quality**: primary
- **angle**: Framework design advice compared: protocols, granularity, interaction patterns
- **claimCount**: 5

### sources 15

- **url**: https://www.erlang.org/doc/system/ports.html
- **quality**: primary
- **angle**: Actors and the outside world: side effects, gateways, external resources
- **claimCount**: 5

### sources 16

- **url**: https://doc.akka.io/docs/akka/current/typed/guide/tutorial_3.html
- **quality**: primary
- **angle**: Actors and the outside world: side effects, gateways, external resources
- **claimCount**: 5

### sources 17

- **url**: https://doc.akka.io/docs/akka/current/typed/persistence.html
- **quality**: primary
- **angle**: Actors and the outside world: side effects, gateways, external resources
- **claimCount**: 5

### sources 18

- **url**: https://temporal.io/blog/idempotency-and-durable-execution
- **quality**: blog
- **angle**: Actors and the outside world: side effects, gateways, external resources
- **claimCount**: 5

### sources 19

- **url**: https://2020.splashcon.org/details/splash-2020-oopsla/90/Actor-Concurrency-Bugs-A-Comprehensive-Study-on-Symptoms-Root-Causes-API-Usages-a
- **quality**: primary
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

### sources 20

- **url**: https://arxiv.org/pdf/1706.07372
- **quality**: primary
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

### sources 21

- **url**: https://ideals.illinois.edu/items/35044
- **quality**: primary
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

### sources 22

- **url**: https://www.microsoft.com/en-us/research/publication/p-safe-asynchronous-event-driven-programming-2/
- **quality**: primary
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

### sources 23

- **url**: https://arxiv.org/pdf/2602.24054
- **quality**: primary
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

### sources 24

- **url**: https://causality.blog/essays/the-isolation-trap/
- **quality**: blog
- **angle**: Critique, empirical bugs, anti-patterns and design verification
- **claimCount**: 5

