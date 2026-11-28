# Design

A caller that starts a flow run and waits for it (`flow-request-target`) needs to know how the run ended. Today a run
has no result, and a `Continue` that fails for good leaves it `running` with nothing armed: no end at all. This change
gives a run its ends: done with an output, cancelled (as today), and failed with a reason. It partly supersedes ADR 0031
and ADR 0044. The picks are mine, under the owner's A1 (2026-11-28). Design reviews 1 and 2 shaped decisions 2 to 7.

## Decisions

1. **`type Output` on `Flow`.** `type Output: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static`, and
   `run` returns `impl Future<Output = Result<Self::Output, HandlerError>>`. A flow without a result writes
   `type Output = ();`, and its body keeps `Ok(())`. Not taken: `step.finish(&value)`. It breaks nothing, but a flow
   can call it twice or forget it, and a caller's `Request<FlowOutcome<O>>` could not be typed from the flow.
2. **The output is saved when the run is done.** `FlowState` becomes `FlowState<I, O = ()>` with `output: Option<O>`,
   which is `Some` only on a done run. Its schema in the contract is the output type's.
   - The output is checked like a step's result, but at most 60 levels deep instead of 61, because
     `flow-request-target` wraps it in `FlowDone`. A value that fails the check is a refusal of the runner (decision 4),
     as a step's result that fails the check already is.
   - A done run always has its output, and `null` is a valid one (for `()` or `None`). Decode reads a done run's
     `output` as `O` from its value, or from `null` when the field is absent, so `flow-request-target` can answer from
     the saved state.
3. **`failed`, an end for the flow code's errors, after the cleanup.** `FlowStatus::Failed`, and `failure:
   Option<String>`, which is `Some` on a failed run, and on a run that is cancelling because its main code failed.
   - **What counts:** only the flow code's errors: an error `run` returns with no latch, and a step closure's error
     latched as `Latch::Failed`, even one the flow catches (the first latch decides).
   - **When it ends the main code:** when the error is permanent, when the message has no retry policy (an operator's
     retry), or when `failures + 1` reaches the attempts of the message's own policy, as in
     `RetryPolicy::retries_after`. `attempt` is not used, because it also counts lost claims.
   - **That turn** succeeds. It keeps the reason, marks the run cancelling and arms a cleanup `Continue` with
     `Flow::retry`, in one commit, exactly as a `Cancel` turn does. The cleanup (`on_cancel`) then runs as for a
     cancelled run. When it succeeds, the run ends `failed`, not cancelled, keeping the reason. A failed main code
     therefore still frees what the run holds (design review 2). With the default `on_cancel` the run is failed one
     turn later. Earlier attempts fail the turn as today.
   - **Cleanup code that fails for good**, by the same rule, ends the run `failed` at once. The reason is
     `cleanup: <error>`, after the main reason and `; ` when the main code failed first.
   - **The reason** has NUL replaced by U+FFFD and is cut at 1,000 characters (on a character boundary, ending in `…`),
     so it can always be stored.
   - **Not taken:**
     - Ending at once with no cleanup. Nothing would free a failed run's resources: a failed run ignores `Cancel`, and
       a flow cannot clean up inside `run`, because the first step error decides the turn.
     - Ending only the runs that have a reply route. Two ways to end would be harder to explain and to test.
4. **Everything else stays as today.** The following fail the turn as before, and the run stays in its phase:
   - the runner's refusals (`Latch::Permanent`: replay mismatches, invalid saved state, values that fail the save check,
     misuse);
   - failures while applying an entry (a registry read, a refused send, a create whose commit is rejected);
   - a killed turn (the maximum turn time) and a panic.

   These come from code, data or infrastructure that an operator can fix, after which a public `Continue` recovers the
   run.
5. **A failed run is final, like done and cancelled.** It ignores `Continue`, `Reply` and `Cancel`, writes nothing, and
   stays until it is deleted. A run cancelling after a main failure ignores `Cancel`, as any cancelling run does. A new job needs a new run key. A host that wants more patience sets `Flow::retry`. Not
   taken: letting `Continue` revive a failed run. A caller that already got a failed outcome (`flow-request-target`)
   would then get a second one.
6. **What breaks: watchers, Rust and the contract** (all under BREAKING in the proposal and the CHANGELOG):
   - **Watchers.** The last attempt's turn succeeds, so its message ends done. A flow-code failure no longer sends a
     `Failed` notice to watchers, leaves no dead letter, and is no turn to retry. The failure is data now: the run's
     `failed` status and reason, and the outcome reply of `flow-request-target`. Watchers still see the failures of
     decision 4.
     - The actors guide's advice to "watch both events and flow types" changes in the handoff.
     - Not taken: a self-watch that turns the core's `Failed` into the status. It reverses the rule that the runner
       installs no watch, needs a notice variant in `FlowMsg`, and catches refusals too.
   - **Rust.**
     - Every `Flow` impl adds `type Output`.
     - `FlowState::new(x)` with nothing to pin `O` stops compiling, as in `event.rs`, its doctest and the examples.
     - An exhaustive match on `FlowStatus` needs the new arm.
   - **Contract.**
     - The state schema gains `output` and `failure` (absent: none), and the status gains `failed`. Decode checks:
       `output` only when done; `failure` only when failed or cancelling; nothing armed on a done or failed run.
     - Every flow type's contract hash changes. That is a `ContractConflict` with nodes on the old code, so the upgrade
       follows `docs/operations.md`, "Changing a contract": a new type name, or a maintenance window. The upgrade note
       says so.
     - The sample service's `flows.onboarding` snapshot changes, so the last task runs E2E.
   - **ADR 0080** records it and partly supersedes ADR 0031 (a failed continuation could be recovered) and ADR 0044 (a
     failed cleanup stayed cancelling, and `on_cancel` ran only after a `Cancel`). Their Status lines say so.
7. **One release with `flow-request-target`.** Both change every flow contract, so they ship in the same version and
   flow contracts break once. The change loop releases 0.26.0 before `flow-ends` lands (T1a: 0.26.0 after
   `reply-route`), so both go into the next version.
8. **Tests** run on virtual time with `TestActors` and the test flow:
   - A run returns an output, and it is saved.
   - A transient step error fails the turns until the last attempt, which starts the cleanup. The run ends failed with
     the reason and nothing armed. A lost claim does not shorten the count, and a `Continue` without a retry policy
     ends the main code on its first failure.
   - A permanent error ends the main code at once.
   - A failed main code runs an `on_cancel` that releases a resource, and the run ends failed.
   - A failing cleanup ends as failed with `cleanup: `, after a `Cancel` and after a main failure.
   - A done run with `type Output = ()` reads back its `null` output.
   - A reason with a NUL and 5,000 characters is stored cleaned and cut.
   - A replay mismatch and an output that fails the save check fail the turn and leave the run running, and a public
     `Continue` after the fix recovers it.
   - A failed run ignores `Continue`, `Reply` and `Cancel`.
   - Decode refuses the broken invariants.
   - The examples' flows get `type Output = ();`, or a real output where it shows something.

## Docs this breaks

Every Rust block with an `impl Flow`, or a `FlowState::new` without a pinned output, stops compiling: the actors
guide's flow blocks, and any in the README, the design guide or the upgrade guide. The builder lists them in
`docs-handoff.md` as blocking, for the docs session to fix before landing. The handoff also asks for:
- the flow statuses text;
- the watch advice (actors guide, near "Watch both events and flow types");
- the flow paragraph of `docs/architecture.md`;
- the upgrade note.
