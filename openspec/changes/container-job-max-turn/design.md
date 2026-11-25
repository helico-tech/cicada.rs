# Design

`ContainerActor` pulls a missing image, creates the container and copies its inputs in one turn, so a slow pull needs
a long turn. Today a host raises the node's `default_max_turn`, which `CICADA_MAX_TURN` / `--max-turn` can lower again
(Factory issue 0020). `Actor::MAX_TURN` already wins over the node's default for one type; the container actor gets
it from its job.

```rust
impl ContainerJob for Tests {
    const TYPE: &'static str = "ci.tests";
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(35 * 60));   // a slow pull fits one turn
    // ...
}
```

## Decisions

1. **A constant on `ContainerJob` with a default.** `const MAX_TURN: Option<Duration> = None` on `ContainerJob`, and
   `impl Actor for ContainerActor<J>` sets `const MAX_TURN: Option<Duration> = J::MAX_TURN`. `None` keeps the node's
   default, as today; an associated constant with a default breaks no implementor, so it is a patch. Not taken: a
   field of `ContainerSettings` (a value at run time; `Actor::MAX_TURN` is a constant, and the runtime reads it at
   registration); the actor adding to the job's value (the job knows its images).
2. **The rules of `Actor::MAX_TURN` hold unchanged.** It wins over the node's default and over `--max-turn`; zero is
   refused when the type is registered, as for any actor. A turn past it ends `failed` (`timeout`) without reaching
   the actor's error counting: the step message's own retry policy runs the step again (7 attempts, waiting 1 to 32 s;
   every step is safe to repeat), and after the last attempt the container waits for the next `Check`, `Start`,
   `Stop` or `Remove`, as for any timed-out step today (design review, blocking 1). The constant is not part of the
   contract, so no hash changes.
3. **The rustdoc says what one turn holds.** The doc of `ContainerJob::MAX_TURN` names the two long turns: the create
   step (a pull of a missing image, the create, the folders and copies in) and the stop, which waits up to the
   settings' `stop_grace` in one turn; the limit belongs above both, or stops time out too (design review,
   blocking 2). It also says what a timeout does (decision 2).

## Tests

- A job with `MAX_TURN` of 35 minutes: `ContainerActor<J>`'s `Actor::MAX_TURN` is that value; a job without it gives
  `None`. On virtual time with `TestActors`, with a test runtime that wraps the fake and sleeps in `create` (the fake
  has no timer and gets no delay in its public API): a create step longer than the node's default but shorter than
  the job's limit ends `done`; a job limit below the step ends the step `failed` (`timeout`), and its retry runs the
  step again without a second container.
- A job with a zero `MAX_TURN` is refused at registration.

## Docs

Nothing breaks. No page covers `cicada-containers` yet (the docs session is writing one); the handoff gives it
`MAX_TURN`, when to set it (above the create step and `stop_grace`) and what a timeout does.
