# Design

## Scope

One behaviour: delayed self-sends start linked traces instead of extending an endless trace. The owner's Opus 5.5
adviser settled the decisions. Three implementation tasks plus finishing checks; no runtime or store semantics change.
ADR 0032 records the durable telemetry header convention. It extends ADR 0018's header-only observer capability.

## Decisions

### A timer is a requested delayed self-send

A send is a timer exactly when its target equals the current actor's address and its requested Delivery is At or
After. This includes After(0), At(now), At(past), and a future At that the commit later clamps to store time. The rule
is independent of send_self_with, send_with to one's own ref, send_to or send_raw; compare canonical ActorAddress.
Immediate self-sends and all sends to other actors are Others. Retry rescheduling is not a new handler send.
Not chosen: comparing resolved deliver_at with a clock (commit timing changes classification), or a type-specific rule.

Flow sleep's at(until) Continue, including zero sleep and an early wake rearm, is a timer. Immediate Continue stays in
the trace. Scheduler Tick, debouncer and batcher deadline Flush are timers; their immediate work and outgoing messages
inherit the timer turn's trace. A rate limiter's delayed forward to another actor still inherits its sender's trace.
No flow/scheduler code or actor contract changes; demonstrate their behaviour in telemetry integration tests.

### Two declarative header maps

Add public observer::Sends { All, Timers, Others } and TurnStart::add_header_for(sends, name, value), accepting the same
Into<String> arguments as add_send_header. The old method delegates to All and keeps its behaviour. Internally keep
timers and others maps: All inserts into both; later calls win in each map. Observers::start combines each map in
observer order, so later observers win independently per kind. A panicking start discards both maps of that observer.
Carry the two maps through Observed and TurnInfo to the existing Turn::with_turn_headers shared by typed/raw sends.
Choose one map there and use the existing merge: explicit exact-name send headers win, remaining names sort, and only
the first that fit under 64 headers are added. Invalid observer names/values are omitted with warnings at addition,
including a prefixed name exceeding 200 characters. Invalid calls do not erase an earlier valid value. No removal API.
Case-sensitive header storage/merging is unchanged; telemetry extraction still accepts case-insensitive names.
Not chosen: per-send callbacks (more observer execution/panic machinery), a core timer marker (core wire semantics),
or replacing all headers on a timer (would discard unrelated observers). Audit add_send_header still reaches both.

### A link context beside the parent context

ActorTelemetry injects its newly created span context once through its configured global or explicit propagator.
For each emitted nonempty (name, value), add name for Others and `cicada-link-` + name for Timers. There is no core
OpenTelemetry dependency and no built-in W3C-only codec. A timer normally carries cicada-link-traceparent rather than
traceparent. Unrelated All headers still reach it. The prefix is a stable convention owned by cicada-otel (ADR 0032).

At receipt, extract the parent from plain headers against Context::new(), as today. Separately extract link context
against Context::new() through a prefix-stripping reader using the same propagator; keys() exposes only prefixed names
with the prefix removed. Lookup matches HeaderReader: exact full name first, then first case-insensitive match in
BTreeMap order. Recognize the prefix case-insensitively. If the extracted SpanContext is valid, give the span builder
one Link before start; otherwise give none. No ambient span is used. The receiver does not classify the message again.
The prefix is opaque to core/HTTP, and callers may deliberately send both parent and link contexts from outside a turn.

An explicit plain traceparent (or another observer's All parent) still parents a timer; a valid prefixed context adds
its link independently. An explicit same-name prefixed field wins over automatic injection by the usual merge. Mixed
case duplicates follow existing exact-first extraction, not a new case-insensitive override guarantee. Malformed or
missing link context, unsupported propagation, or header-cap omissions lose the link without failing the message;
plain-parent extraction still applies. No fallback to parenting from link headers. The all-zero workaround remains a
root and can now retain a link. Multiple propagator formats describe at most one extracted link, not one per format.
The saved span context is reinjected, never the incoming header map; arbitrary incoming headers are not forwarded.
Current code builds a span-only context and drops extracted baggage. This change keeps that behaviour.
Not chosen: hand-writing a W3C link field (ignores custom propagators), or reinterpreting a plain parent using a marker
(would override a parent's explicit choice). Ordinary turns still emit the same span attributes, outcome and metrics.

### Attempts and upgrades

Policy retries and lost-claim re-execution reuse the incoming headers: each timer attempt starts a fresh root with
the same sender link, unless plain parent headers were explicit. No deterministic trace id or extra attempt state.
Operator retry uses a new message id but copies headers, so the same rule applies; deliver-now changes no headers.
Dead-letter redrive sends with fresh turn context rather than copying the failed envelope's telemetry headers.
Watch notifications have no observer headers. Nodes without ActorTelemetry add no telemetry headers.

Old queued timers with plain headers keep their old parent. Their first rearm on an upgraded node breaks the chain.
An old receiver ignores link headers and starts a root without a link. A node using a different propagator may lose
the link, just as it can lose a parent today. No migration, restart barrier or contract hash change is required.
Parent-based sampling now treats ordinary timer attempts as roots; the configured sampler decides whether to export
them and their links. We add no sampling policy. More traces and less ancestry are the intended observable change.

### Verification and documentation

Use virtual-time TestActors and the SDK's in-memory exporter. Assert trace ids, parent ids and link contexts, including
rearming chains, explicit parents, retry attempts, zero/past delivery, header caps, case aliases and a custom propagator.
Core observer/send-header tests cover both maps, every send path, call/observer order and panic/limit isolation.
Add flow sleep and scheduler coverage in cicada-otel tests with a dev dependency on cicada-actors; no production cycle.
Update architecture, core guide/doctests, actor-guide trace boundaries and crate docs when built, plus the changelog.
No implementation or runtime test execution belongs to this design loop.

Review 1: ready for tasks, no blockers. Corrected the proposal's restart claim: persisted parent headers survive a
node restart, so only rearming with the new telemetry convention breaks that old chain.
