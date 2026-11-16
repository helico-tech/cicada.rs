# Design review resolution

Opus 5.5 returned not yet for sample-worker-mode.design.2 on 2026-11-16. The standing review session had claimed
design.1 first and returned ready with one Docker note; the owner-requested Opus review governs this resolution.
The Opus reviewer replaced its own overlength report with the 299-word report; verdict/findings were unchanged.
The owner's autonomy delegation covers the split, decisions, tasks and ready approval, including this batch's queue.

- Resolve the size blocker by splitting the independent HeartbeatWatch catalog, snapshots and smoke into the next
  change, sample-heartbeat-watch. Keep actual executor identity with HTTP execution: truthful worker records need
  it. The five tasks now separate identity, App definitions, HTTP checks, Docker acceptance and handoff/release.
- Align Impact with the selected sample-service/src/bin binary, not a possible example binary.
- Remove the redundant Sample node_config override and identity startup guard. App already applies flag and env
  after its host default (settings.rs:414–422), so the override was not itself a precedence bug. Removing it leaves
  one owner of resolution and default generation. The public helper can remain for source compatibility.
- Use CICADA_TOKEN in deployment/E2E commands; API_TOKEN still protects the server. No App token-policy change.
- Do not remove public constructor parameters. Both ports' GitHub default trees currently contain only README and
  icon assets, not their Rust crates, and no local checkout was found in the project roots checked. They therefore
  do not establish absence of consumers. Preserve signatures and explicitly ignore their legacy label arguments;
  prove differing legacy and runtime ids use runtime identity. No claim that port-specific builds were checked.
- The builder writes a handoff and verifies the docs session's prose has landed; it does not edit handwritten docs.
- Apply design.1's volume note: no supplied/shared database volume is different from no anonymous image volume.
  Keep the inaccessible-store assertion and remove anonymous volumes during container cleanup.

This follows the review's exact scope reduction and makes compatibility stricter, without changing the executor
method; no further executor review is needed. The split catalog change gets its own Opus review. No runtime, Docker,
port integration or future predecessor implementation has been validated by these design-stage checks.
