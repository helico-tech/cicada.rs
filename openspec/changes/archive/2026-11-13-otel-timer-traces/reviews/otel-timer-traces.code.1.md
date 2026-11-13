---
id: otel-timer-traces.code.1
kind: code
change: otel-timer-traces
round: 1
base: 9662c2c43c5e885eb69e0548d76ff4f7b11f2fe9
head: 4213c2382d68310f1229736bd0b06b18a1e2c6f9
requested: 2026-11-13T15:32:56+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done otel-timer-traces.code.1 <file>`.

## Note from the requester

Version: 0.16.8

Builder report (round 1)

otel-timer-traces is built in full, version 0.16.8, on branch feat/otel-timer-traces (4 commits on top of 9662c2c). Nothing pushed, merged, tagged or archived.

Tasks
- Task 1, done: observer::Sends { All, Timers, Others } and TurnStart::add_header_for; add_send_header now means All. Observers keep two header maps, one for timers and one for other sends; every send picks one at the shared merge in turn.rs. Seven new core tests.
- Task 2, done: ActorTelemetry puts plain headers on other sends and cicada-link- headers on timers. A turn reads its parent from the plain headers and at most one link from the prefixed ones. 21 new tests. cicada-actors is now a dev dependency of cicada-otel.
- Task 3, done: crate docs, architecture (new Timer term, telemetry note), core guide (with a doctest), actors guide (flow sleep and scheduler), changelog and version; docs/openapi.json regenerated for the version.
- Task 4, done: finishing checks.

Checks
- cargo fmt --all --check: passed.
- cargo clippy --workspace --all-targets -- -D warnings: no warnings.
- cargo test --workspace with PostgreSQL: 2327 passed, 0 failed.
- openspec validate otel-timer-traces --strict: valid.
- Not run: the Docker end-to-end tests.

Departures and open questions
- No design changes.
- One spec scenario is not tested: an older node reading the new link headers, because that needs old code.
- The lost-claim test abandons the claim with claim_and_abandon. The re-run is checked to be a linked root; the abandoned claim itself makes no span.

New concepts
- Timer: a message an actor sends itself with a delay or a set time, even a zero delay or a past time. Every other send is one of the "others". The sending method and the actor's type do not matter.
- Linked timer trace: with ActorTelemetry, a timer's turn starts a new trace with a link back to the turn that set it, instead of joining that turn's trace. The link travels in headers like cicada-link-traceparent. A plain traceparent set on the send still becomes the parent.

Breaking changes
- No API breaks.
- Traces change on purpose: timer turns, flow sleeps and scheduler ticks become roots. A parent-based sampler no longer follows the sender's choice for them. (Trace shape is not on the README's list of breaking changes, so the version is a patch.)
