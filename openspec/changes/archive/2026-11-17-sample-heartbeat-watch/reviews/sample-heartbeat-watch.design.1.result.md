Verdict: ready for tasks

## Blocking findings

None. One behaviour (one catalog entry), three tasks. It matches the code: `CatalogType::of` in `sample-service/src/catalog.rs`, the `cicada.heartbeat-watch` contract and reference, both named paused tests, `CounterMsg::Step`, and the existing `sample-execution` spec. `openspec validate` passes.

## Notes

1. design.md §3: the check "sample contract equals `contract_of::<HeartbeatWatchActor>()`" is always true, because `CatalogType::of` calls `contract_of::<A>`. Stronger evidence: compare the new sample snapshot's hash with `docs/reference/contracts/cicada.heartbeat-watch.json`. Or drop the check, since the snapshot and the digest already pin the contract.
2. design.md §5 and the predecessor's §9 both hand off prose for the same `docs/guide/app.md` "Sample service" block, and the predecessor rewrites its node-id sentence. Say this handoff builds on the predecessor's landed prose, so the docs session does not merge two stale edits.
3. design.md §4 / spec "A caller explicitly starts a round": "inspect waiting/no timer" over HTTP. Name the read: actor state plus `GET /actors/{type}/{key}/messages` for pending messages. That keeps "no armed timer" testable through both hostings.
4. The spec says the round "works through the HTTP executor", but §4 runs the in-process harness with no Docker class. Add one sentence to the spec saying this smoke runs in process, so nobody expects container evidence.
5. proposal.md Why cites "decision 34" with no link. Point to where it is recorded, or drop it.

## Owner questions

None.

## Checked / not checked

Checked: the catalog, contract snapshot and digest tests, sample state/message samples, the HeartbeatWatch actor API and its tests, the app.md heading, the actors guide's Heartbeat watch heading, predecessor design, earlier changes' sample scope (node-down-alert has no sample wiring), and the OpenAPI routes.
Not checked: builds and tests (none run), the consumer skill's route lists, the predecessor's unbuilt harness API, CHANGELOG/release mechanics.
