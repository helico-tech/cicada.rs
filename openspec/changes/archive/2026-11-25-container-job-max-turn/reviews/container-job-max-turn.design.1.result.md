Verdict: not yet

## Blocking

1. **Timeout path: misdescribed, and the spec has no scenario.** design.md Decision 2 says "the actor's own retry handles it". It does not. The actor's error counting and backoff never see a timeout (mod.rs: "Runtime errors ... never fail a turn"). A timed-out `Poll` is retried by its message `retry_policy()`: 7 attempts, 1-32 s. After that the poll is dead, `armed` still points at it, and the container stalls until a `Check`, `Start`, `Stop` or `Remove`. The design's Tests list the timeout and the zero refusal, but spec.md has neither. Fix: correct Decision 2. Add two scenarios: (a) the job's limit is below the create's length, so the poll ends `failed` (`timeout`), is retried, and the step runs again safely (create keeps an existing container); (b) a zero `MAX_TURN` is refused at registration.
2. **Decision 3 contradicts the existing rustdoc.** It says "every other turn is short". But a stop's `stop_grace` (a host setting) also runs inside one turn, as `ContainerActor`'s doc already says. A job limit below `stop_grace` makes stops time out. Fix: the doc of `ContainerJob::MAX_TURN` names both the create (pull, create, inputs) and the stop grace, and says to set the limit above both.

## Notes

1. The fake "answers at once: it uses no timer", so it cannot give a 20-minute create. State that the test wraps `FakeRuntime` in a test-local runtime that sleeps in `create`. Do not add a delay to the fake's public API.
2. Docs: no container page exists in `docs/` (only ADRs 0056, 0057 and 0062). Name where the handoff's prose goes. `docs/guide/core.md:333` says a type can override the default with `MAX_TURN`, which stays true.
3. Checked and correct: the contract holds no `MAX_TURN` (`Contract::new`), so no hash changes. `--max-turn` / `CICADA_MAX_TURN` only set `default_max_turn`. The worker path (`worker.rs:357`) also honours `Actor::MAX_TURN`. No clash with `container-logs-unreadable`.

## Questions for the owner

None.

## Not checked

I ran no tests or builds. I did not check that `tokio::time::timeout` with a sleeping runtime wrapper behaves as expected under `TestActors`.
