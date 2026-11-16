# Design

Split from sample-worker-mode on Opus 5.5's size finding, under the owner's delegation. This change follows that
executor and only adds an existing actor to its shared catalog. No new ADR, behavior, schema or wire contract.

1. **One catalog entry.** Add CatalogType::of(HeartbeatWatchActor) in sample-service/src/catalog.rs alongside the
   other ready-made actors. Both Sample::new and Sample::executor already use this catalog. App's all selection
   includes cicada.heartbeat-watch; selecting that exact type works; none still hosts nothing. No aliases or groups.
   Reuse the existing actor directly, not a sample wrapper with different state/messages or another type name.
2. **No automatic instance.** Do not add a Singleton, Beat, startup hook or native node heartbeat. The caller creates
   HeartbeatWatch::named(key) with HeartbeatWatchState::new(silence, target, alarm), ensures the target exists and
   sends Beat to begin a round. This monitors handled Beats, not the node table, physical health or wall-clock receipt.
   A server retaining the original sample bootstrap still creates only its already-defined singletons/watches.
   NodeDownMonitor remains excluded because it requires an injected source; do not claim the sample hosts every
   ready-made actor. No sample timer/deadline/retry policy overrides and no automatic use of the dead-letter actor.
3. **Contract evidence.** Add sample-service/tests/contracts/cicada.heartbeat-watch.json and its entry in
   tests/contract-texts.json through CICADA_UPDATE_CONTRACTS=true with the sample contract_snapshots test. Review
   the new snapshot against the existing docs/reference/contracts/cicada.heartbeat-watch.json; their parsed contract
   JSON must match. The snapshot and digest tests pin future changes; no catalog-versus-constructor test is needed.
   Existing sample entries and the ready-made reference must stay byte-for-byte fixed. Reuse cicada-actors' existing
   wire/state samples; do not duplicate an archive or portable fixture.
4. **Catalog and transport checks.** Extend sample tests to assert presence under all, exact-name selection and
   absence under none, for both definitions, plus no heartbeat actor instance after startup. Use the predecessor's
   send-only-server/executor harness for a bounded HTTP smoke: create a Counter alarm target and an explicit watch
   with a one-hour silence and CounterMsg::Step alarm. Before Beat, inspect the actor state (waiting, no armed id)
   and GET /actors/{type}/{key}/messages (no pending messages). Send Beat, then read alive with lastBeatAt and armed
   set, and a pending Expire matching that id. Check the same sequence through the original sample's direct-store
   hosting. Both use in-process App/HTTP transport, not a new container test. This proves
   registration, schema acceptance and dispatch; it does not wait for a real silence deadline or assert elapsed time.
   Reuse paused TestActors tests a_created_watch_waits_and_arms_nothing_until_its_first_beat and
   a_silence_sends_one_alarm_and_arms_nothing_more in cicada-actors/tests/heartbeat_watch.rs for timing/alarm behavior.
   No new actor algorithm tests, Docker class or service: existing catalog and image acceptance run in the full gate.
5. **Docs ownership.** Write docs-handoff.md asking the docs session to add the sample's heartbeat availability and
   explicit create/Beat duty to docs/guide/app.md's Sample service block, building on sample-worker-mode's already
   landed prose rather than replacing it with an older block. Link to the existing actors guide
   Heartbeat watch section. Keep the existing heartbeat example and reference as the source of behavior. No new
   guide heading, worked example, architecture claim or guarantee row is needed. Verify the docs session's prose has
   landed before landing; builders do not edit handwritten docs. Check docs/doctests and both skills' existing routes.
6. **Delivery.** Three implementation tasks cover catalog/contracts, targeted verification and handoff/release
   preparation, then the normal committed PostgreSQL/Docker gate. The image automatically includes the new catalog
   entry when rebuilt. No runtime or container execution is claimed by design validation.
