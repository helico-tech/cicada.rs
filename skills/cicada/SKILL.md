---
name: cicada
description: Build, test and operate Rust projects that use Cicada, the durable actor library on a SQL store (crates cicada, cicada-test, cicada-axum, cicada-client, cicada-app, cicada-actors, cicada-otel). Use it when a project depends on these crates, or the user asks to add Cicada, design actor types or messages, write durable flows, call external services from actors, test with TestActors, serve the HTTP API, run workers, or deploy and upgrade nodes. It finds the exact Cicada source the project resolves and routes to that version's guides, examples and code.
metadata:
  cicada-version: 0.19.2
---

# Cicada

This skill routes; it does not teach Cicada from memory. The guides, examples and code of the Cicada source that the
project actually resolves are the authority for that project. They may be older or newer than this skill, or a fork.

## 1. Find the project's Cicada source

Run the helper from the project folder. It is read-only and offline: it runs `cargo metadata --locked --offline` for
one target and feature selection, and never fetches, installs, writes a lockfile or changes the project.

```sh
python3 <this skill's folder>/scripts/cicada_source.py            # host target, default features
python3 <this skill's folder>/scripts/cicada_source.py --manifest-path crates/api/Cargo.toml \
    --target x86_64-unknown-linux-gnu --no-default-features --features postgres
```

Replace `<this skill's folder>` with the actual path of the folder that holds this SKILL.md, wherever it is installed
(a Claude Code plugin's copy or an unpacked release archive), quoted for the shell, such as
`python3 "/path/to/cicada/scripts/cicada_source.py"`. The current folder stays the project's.

It prints one JSON report. Act on `status`:

| `status` | What to do |
| --- | --- |
| `ok` | One source. Read every route below beneath its group's `root`. |
| `self` | The project is the Cicada repository. Go to [Working on Cicada itself](#working-on-cicada-itself). |
| `mixed` | Several versions or sources (forks, patches, two tags). Show the groups, and ask which package and configuration the task is about before advice that assumes one source. Work that does not depend on the choice can go on. |
| `no-docs` | The package is resolved but its tree has no docs (vendored, reshaped). Read that exact package's source at its `manifest_path`, or its rustdoc. Do not conclude that Cicada is absent. |
| `none` | No Cicada crate in this selection. Check another `--manifest-path`, `--target` or `--features` before you say so; for a new integration see section 5. |
| `unavailable` | Read `reason` and `diagnostic`. Use the manual path below. A `hint` may suggest `cargo fetch --locked`: that downloads, so it needs the user's task or consent; never `cargo update`. |

Each group reports `version`, `source` (the full Cargo source id), `selector` and `revision` for git, `commit` and
`dirty` of the checkout on disk, `docs_available`, `version_relation` (the project's version against this skill's:
`older`, `equal`, `newer`, `unknown`) and `revision_match` (`same`, `different`, `unknown`, against the release commit
in this skill's `SOURCE.json`). Equal versions are not the same code: only `revision_match: same` with `modified:
false` is. `unknown` and `null` mean the checkout could not be read (no git, a failed probe, no `SOURCE.json`): treat
them as not proven, never as a match. Say once when the project is older or newer than the skill, then follow the project's source. The report
names the one target and feature selection it examined; it does not cover other configurations.

**Manual path** (no Python, no toolchain, Cargo failed): read `Cargo.lock` for the `name`, `version` and `source` of
each Cicada crate, and the manifests for features. Read source only where it verifiably is: a `path` dependency's
folder, or the checkout Cargo reports. Do not guess `~/.cargo` folder names or shortened hashes. Say what you could not
verify, and go on with the work the local evidence supports.

## 2. Read routes in that source

Every route below is a path from the root of the Cicada source in the report, never from this skill's folder. Read
only the routes of the task. The routes are written for this skill's version: an older or newer source may lack a
file, such as a worked example added later. If a file or heading is missing (headings move), search that source's `README.md`,
`docs/` and public code for the function. Call a feature unsupported only with evidence; otherwise say it is
unverified there. Never fill a gap from Cicada's latest `main`, a newer release or a proposal: the project builds
against its own source. A `build`, `doc` or `test` run that the task needs is part of that task, with its normal
effects; it is not part of the read-only discovery.

## 3. Keep in mind

These hold in current Cicada; check the details for the version in hand.

- An actor is a consistency boundary: its turns never overlap, and a turn's state, sends and creates commit together.
  Size actors by invariant and key them by a stable identity (`docs/guide/core.md#design-an-actor`).
- Effects inside the system happen once; handlers and calls to outside services run at least once. Give each outside
  call a stable idempotency key: the message id, a flow step's key, or a business id (`docs/guide/core.md#retry`).
- Retry is chosen by the sender of a message; a handler failure without a policy ends the message `failed`. Lost
  claims are capped separately by the claim limit (`docs/guide/core.md#claim-limit`).
- Cancelling a running turn is cooperative: a handler stops at its next `.await`; blocking code runs on. Stopping a
  business process, such as a flow run, is another thing: look for what the source offers (deleting or pausing an
  actor, cancelling a pending message) in `docs/guide/core.md#control-effects`.
- Wait durably with a delayed message or a flow's sleep, not by holding a turn open. A bounded await inside a turn is
  fine.
- A contract (the schemas of state and messages) must be equal to the registered one for a node to start. Reading old
  state with serde defaults is not enough; pin contracts with snapshots and follow the release's upgrade notes.
- Test on virtual time: `#[tokio::test(start_paused = true)]` with `TestActors`.
- The HTTP API has no authentication of its own; the host adds it.
- Add only the crates and features the task needs.

## 4. Routes

| Task | Read in the source |
| --- | --- |
| Choose layers, crates and features; add Cicada | `README.md#layers`, `README.md#quick-start`, `README.md#using-a-release-in-another-project`, `docs/how-to/install.md#choose-the-crates`, `docs/how-to/install.md#authenticate-cargo`, `CHANGELOG.md` |
| Understand example message paths visually | `docs/guide/patterns.md` |
| Behaviour and guarantees | `docs/architecture.md#guarantees`, `docs/architecture.md#turn-semantics`, `docs/architecture.md#known-limitations`, `docs/guarantees.md` |
| Actor design: boundaries, keys, state, messages | `docs/guide/design.md#from-problem-to-design`, `docs/guide/design.md#choose-actors-and-identities`, `docs/guide/design.md#design-protocols`, `docs/guide/design.md#coordinate-work-and-time`, `docs/guide/design.md#recover-from-failure`, `docs/guide/design.md#choose-a-pattern`, `docs/guide/design.md#recognize-design-mistakes`, `docs/guide/core.md#define-an-actor-type`, `docs/guide/core.md#design-an-actor`, `docs/guide/core.md#how-deep-a-value-can-be`, `examples/src/sync.rs`, `examples/src/fanout.rs`, `examples/src/registry.rs`, `examples/src/singleton.rs` |
| A socket, process or device handle held by a host beside durable actor state | `docs/guide/design.md#work-with-outside-systems`, `docs/guide/design.md#a-websocket-session`, `docs/guide/core.md#follow-changes`, `examples/src/websocket_session.rs`, `examples/src/websocket_session/` |
| Children, watches, control effects | `docs/guide/core.md#child-actors`, `docs/guide/core.md#watches`, `docs/guide/core.md#control-effects`, `examples/src/supervision.rs` |
| External calls, idempotency, retry, sagas | `docs/guide/core.md#retry`, `docs/guide/core.md#claim-limit`, `docs/guide/core.md#message-persistence`, `examples/src/saga/`, `examples/src/steps.rs` |
| Timers, schedules, request and reply | `docs/guide/core.md#timers`, `examples/src/timer.rs`, `examples/src/cron.rs`, `examples/src/reply/` |
| Durable flows: steps, keys, replay, sleep, requests and waits, stopping a run | `docs/guide/actors.md#flows`, `docs/guide/core.md#control-effects`, `docs/guide/actors.md#durable-sleep`, `docs/guide/actors.md#event`, `examples/src/flow_onboarding.rs`, `examples/src/flow_events.rs`, `examples/src/flow_approval.rs`, `examples/src/flow_resources.rs` |
| Ready-made actors | `docs/guide/actors.md`, `docs/guide/core.md#watches`, `docs/guide/actors.md#supervisor`, `docs/guide/actors.md#dead-letters`, `docs/guide/actors.md#topic`, `docs/guide/actors.md#scheduler`, `docs/guide/actors.md#rate-limiter`, `docs/guide/actors.md#debouncer`, `docs/guide/actors.md#batcher`, `docs/guide/actors.md#heartbeat-watch`, `docs/reference/contracts/`, `examples/src/rate_limit.rs`, `examples/src/debounce.rs`, `examples/src/batching.rs`, `examples/src/heartbeat.rs`, `examples/src/dead_letters_upgrade.rs` |
| Store, node, reading actors and changes | `docs/guide/core.md#open-a-store-and-run-a-node`, `docs/reference/stores.md`, `docs/guide/core.md#list-actors-with-their-state`, `docs/guide/core.md#follow-changes`, `docs/guide/core.md#configuration` |
| Testing | `docs/guide/core.md#testing-with-testactors`, `docs/guide/core.md#contract-snapshots`, `docs/guide/core.md#state-samples`, `examples/tests/` |
| HTTP API, authentication, client | `docs/guide/http.md#run-it-in-an-axum-service`, `docs/guide/http.md#http-api`, `docs/guide/http.md#errors-and-limits`, `docs/guide/http.md#the-schema-check`, `docs/guide/http.md#the-client`, `docs/architecture.md#http-contract`, `docs/openapi.json` |
| Workers and leased turns | `docs/guide/core.md#run-actors-in-a-worker`, `docs/guide/core.md#lease-turns-for-an-executor-outside-the-node`, `docs/guide/core.md#register-a-contract-without-hosting`, `docs/guide/http.md#run-a-worker-over-http`, `docs/architecture.md#workers`, `docs/operations.md#external-leases`, `examples/src/remote_worker.rs` |
| The app binary and command line | `docs/guide/app.md#run-it-as-a-binary`, `docs/guide/app.md#a-worker-over-http`, `docs/guide/app.md#command-line-and-terminal-ui`, `docs/reference/cli.md`, `docs/guide/app.md#sample-service`, `sample-service/` |
| Telemetry and logs | `docs/guide/core.md#observers`, `docs/guide/core.md#opentelemetry`, `docs/operations.md#telemetry`, `docs/operations.md#logs` |
| Deploy, size, tune | `docs/operations.md#sizing`, `docs/operations.md#deploying`, `docs/operations.md#shutdown-and-probes`, `docs/operations.md#streams-and-proxies`, `docs/operations.md#destructive-controls`, `docs/operations.md#store-tuning`, `docs/operations.md#tls-for-postgresql`, `docs/operations.md#on-call`, `docs/operations.md#the-claim-limit` |
| Contract changes and upgrades | `docs/operations.md#changing-a-contract`, `docs/operations.md#upgrading`, `docs/how-to/upgrading.md`, `docs/how-to/install.md#upgrade`, `docs/guide/core.md#the-contract-of-a-type`, `CHANGELOG.md` |
| An API not named here | the crate's public code under `crates/`, and its rustdoc |

## 5. A new integration

Recommend one published stable release for every Cicada crate, with explicit features for the chosen layer and store
(`docs/how-to/install.md#choose-the-crates` and `README.md#using-a-release-in-another-project` of that release). A tag alone does not prove a release: a release has
a GitHub Release. Offline, this skill's own version is a candidate, not proof that it is the latest. Keep a user's
explicit choice of a commit, a fork or a path; do not replace it. Edit manifests and fetch dependencies only as far as
the user's task asks. A crate added to an existing integration comes from the same source and version as the others;
the edit changes `Cargo.lock`, so the helper reports `unavailable` until Cargo has updated the lock as part of that
task.

## 6. Verify

Run the project's own tests: actor behaviour on virtual time with `TestActors`. When the task adds or changes a type,
add or update its contract snapshot, and state samples for states that must stay readable. Say which checks you ran
and which you did not.

## 7. Update this skill

This skill is installed in one of two ways. Updating either never changes the project's Cicada dependency.

- **Claude Code plugin** (`cicada@cicada`, used as `/cicada:cicada`). Claude Code manages it: update it through
  `/plugin`. The plugin follows Cicada's `main` and is fetched again when its version changes, so it can hold
  documentation changed after the release tag; it has no `SOURCE.json`, so `revision_match` stays `unknown`. Edits to
  the plugin's copy do not survive an update.
- **Release archive.** A release of Cicada attaches `cicada-skill-vX.Y.Z.tar.gz`. To update, unpack it into a staging
  folder, compare it with the installed copy, and replace the copy only after the user has seen local changes. Nothing
  updates it on its own.

A user may keep an archive installation. To switch to the plugin, first compare the installed copy with the plugin's
and keep the user's local changes; only then remove the old copy, so that one copy is active. Never switch, uninstall
or turn on automatic updates without the user. Releases before the skill existed have no archive: use a newer skill
with the project's own older source, and say so.

## Working on Cicada itself

Only when the helper reports `self`: this is the Cicada repository, not a consumer. Follow its own instructions in
`AGENTS.md` (build, test, rules, and the change loops under `openspec/` and `.claude/skills/`). They do not apply to
projects that merely use Cicada.
