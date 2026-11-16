# Design review resolution

Opus 5.5 returned ready for tasks for node-down-alert.design.1 on 2026-11-16; no blocking findings or owner questions.
The reviewer shortened its own submitted report to 292 words. This resolution uses that final report. Task/design
approval is delegated by the owner, including exceeding the usual ready-queue limit for this batch.

- Keep whole-snapshot refusal deliberately. Partial samples must not clear dedup history or imply absent nodes.
  One invalid row, clock regression or >1000 rows suppresses all alerts for that check; polling continues and
  lastProblem stays visible until a successful sample. Add explicit operational guidance and assertions.
- Retain ActorSystem::nodes as the initial source rather than expanding core scope with a lightweight read API.
  Disclose its full running-turn read and transient allocation cost, including potentially indefinite timeout failure.
- Add monitor Failed/Stopped watching to the checked example and handoff. Explain that caught read errors instead
  update lastProblem and need separate inspection; repair plus Rearm restores a terminally failed/lost poll.
- Name initial retained-dead-row bursts and repeated flapping episodes without rate limiting in operations guidance.
- Keep the 1000-row cap fixed and visible. It bounds stored work, not the source's query memory or cluster size.

These clarify intentional limits without changing the method or wire contract; no second review is needed.
Strict validation/docs checks cover artifacts; Topic behavior, concurrency, fault recovery and source cost remain
implementation verification work, without a universal scalability claim.
