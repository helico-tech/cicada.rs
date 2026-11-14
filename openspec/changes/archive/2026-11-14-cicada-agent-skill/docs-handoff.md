# Docs handoff for cicada-agent-skill

The owner split the work on 2026-11-14: the docs session owns `README.md` and the hand-written docs under `docs/`, and
this change keeps the code. This file lists the prose the change needs from the docs session. Each part gives where it
goes in the new layout and the agreed text or its substance. The text below is complete, so this file stands alone.
The original edits are also in two scratchpad patches of the build session, which may be cleared:
`cas-prose-committed.patch` (the branch's README, guide, operations and AGENTS.md edits) and `review1-docs.patch`
(review fixes 2 and 3).

What the code branch now holds: `skills/cicada` (the skill and its helper), its tests (`examples/tests/skill.rs`,
`examples/tests/markdown/mod.rs`, `examples/tests/skill/`), `scripts/package-skill.py`, the release workflow and
`scripts/release-check.sh`, the changelog entry and version 0.17.2. Its AGENTS.md lines (a test command, two map rows
and a rule about the skill) were committed on the branch; the hook allowed them, so nothing of AGENTS.md is handed
over.

## 1. Actor design in the core guide

**Where.** `docs/guide/core.md`, the section `### Design an actor` (under "Define an actor type"). The new section
there is short; extend it with the substance below, keeping its heading or renaming it. If the heading changes, also
change the route in `skills/cicada/SKILL.md` (see part 5).

**Substance, agreed in review.** Each point follows from the guarantees and names the worked example that shows it:

- **Draw the boundary around what must change together.** The turns of one actor never overlap (G1), and a turn
  commits its state, sends, creates and outcome together (G2). Keep the data of one invariant in one actor: a balance
  and its holds, an order and its lines. Data that changes on its own belongs to another actor, and an invariant across
  actors holds only eventually, through messages (`examples/src/saga/`). Neither one actor for everything nor one actor
  per request is a rule; the invariant decides.
- **Key it by a stable identity.** The key is how every sender reaches the actor, and a flow's step keys contain it.
  Use an id that does not change (a tenant or order id), not a display name or the time of a request. A key whose name
  can come back (an address reused after a delete) needs a part that tells the lives apart. Many instances spread the
  work over the nodes; a singleton (`examples/src/singleton.rs`) runs everything one turn at a time. Find instances
  again by prefix (List actors with their state) or with a registry (`examples/src/registry.rs`).
- **Keep state and turns bounded.** Every turn loads and rewrites the whole state. Keep the last N items or a count,
  give each item a child (`examples/src/fanout.rs`), or keep large data elsewhere with a reference in the state. Do
  paged work one page per turn (`examples/src/sync.rs`): each turn short, `MAX_TURN` small, a retry repeats one page.
- **Make a message carry what its handler needs.** A command or a fact with the data the receiver uses, not a pointer
  into the sender's state. A reply goes to `message().from` or to a ref the message names (`examples/src/reply/`).
  A new variant or field changes the contract.
- **Internal effects happen once; external ones at least once.** State, sends, creates, cancels and control effects
  commit together or not at all, and are never applied twice (G4). The handler can run more than once for one
  message. **Corrected text (review fix 2):** "A crash or a lost lease (an abandoned claim) runs it again, and so does
  a retry the sender asked for. A turn cut off at its maximum duration runs again only under such a retry policy, and
  a handler that returned only after its maximum (blocking code) ends `failed` and is never run again. A run that did
  not commit leaves no effect inside the system, but its call to a service outside (a payment, a mail) may have
  happened, so that call can happen twice." This matches the turn-outcome table of `docs/architecture.md` and the test
  `a_turn_timeout_ends_failed_timeout_by_default`. Then: give the call an idempotency key that is the same on every
  run: `turn.message().id` for the work of one message, the key `step.run` gives a flow step, or a business id when
  the service must refuse a second charge for the same order across messages. A key made from the attempt, the clock
  or a random number protects nothing. When a later effect must not repeat an earlier call, make the call its own step
  (`examples/src/steps.rs`).
- **Wait with a message, not in a handler.** A durable wait is a delayed send (Timers) or a flow's `step.sleep`. A
  bounded await (a request with a timeout) within `MAX_TURN` is fine; do not keep a turn open to let time pass or to
  wait for another actor. A cancelled turn stops at its next `.await`; code in `spawn_blocking` runs on, so its effect
  can land after the turn ended without a commit.
- **Reading old state is not an equal contract.** `#[serde(default)]` lets new code read old states, and state samples
  prove it. The contract still changes, and a node refuses to start while its hash differs from the registered one,
  also for such a compatible field. Deploy it as a contract change (a versioned type name, or a stop of the world, per
  "Changing a contract"), never by deleting a registration only because the new type decodes the old data. The order
  of a deploy follows the release's changelog and upgrade notes. Pin each contract with a snapshot.
- **Look for a ready-made actor first**, where it fits; it needs the crate `cicada-actors`.

## 2. Installing and updating the skill

**Where.** The README's place for it, or `docs/how-to/install.md` with a link from the README and `docs/README.md`
("Work with an agent"). `docs/agents.md` may link to it too.

**Substance.** `skills/cicada` is a skill for a coding agent in a project that uses Cicada. It does not repeat the
guides: it finds the Cicada source the project resolves and sends the agent to that source's guides, examples and code.

- **Install.** Each release from 0.17.2 on attaches `cicada-skill-vX.Y.Z.tar.gz` to its GitHub Release. Unpack it into
  a staging folder, read it, and copy its `cicada/` folder into the agent's project skill folder, such as
  `.claude/skills/cicada` for Claude Code. **Agreed commands (review fix 3):**

  ```sh
  staging=$(mktemp -d)
  gh release download v0.17.2 --repo helico-tech/cicada.rs --pattern 'cicada-skill-*.tar.gz' --dir "$staging"
  tar -xzf "$staging/cicada-skill-v0.17.2.tar.gz" -C "$staging"
  mkdir -p .claude/skills
  cp -R "$staging/cicada" .claude/skills/
  ```

- **What it needs.** Python 3.11 or newer, the project's Cargo and rustc, and git. The helper runs
  `cargo metadata --locked --offline` for one target and feature selection, reads nothing from the network and writes
  nothing (no lockfile, no download, no toolchain). A dependency not yet downloaded is reported with `cargo fetch
  --locked` as a hint the agent does not run on its own. Without Python or a toolchain the skill says how to read
  `Cargo.lock` and local sources by hand.
- **Which source.** The helper reports each source group: version, full Cargo source, the checkout's commit and local
  changes, and whether docs are there. Two sources are reported as such and the agent asks which one the task is about.
  An equal version with another commit or local changes is not the same code, and a checkout git cannot read is
  reported as unknown, never as a match. The project's own source always wins over the skill's age.
- **Update.** Nothing updates an installed copy on its own. Unpack the new archive into a new staging folder as above,
  compare, keep any local change you want, and only then replace the folder whole, so no old file stays behind:

  ```sh
  diff -r "$staging/cicada" .claude/skills/cicada   # the changes, yours included
  rm -rf .claude/skills/cicada
  cp -R "$staging/cicada" .claude/skills/
  ```

  Updating the skill does not change the project's Cicada dependency, nor the other way round. Releases before 0.17.2
  have no archive: a project on one of them can use a newer skill, which reads the project's own older source.

**Verified.** The build session ran both command blocks in an empty temporary project, with the release archive
built locally in place of `gh release download`: the first install gave exactly `SKILL.md`, `SOURCE.json` and
`scripts/cicada_source.py`; after a local edit and an extra file, the update's diff showed both, and afterwards there
was no nested `cicada/cicada`, no stale file, and the copy was identical to the archive.

Also add a row for the skill to the README's documentation table, if the new README keeps one.

## 3. Releasing

**Where.** `docs/how-to/releasing.md` (and the Releasing section of `docs/operations.md` if it still holds steps).

**Substance.**

- When setting the version, also set `metadata.cicada-version` in `skills/cicada/SKILL.md`;
  `cargo test -p cicada-examples --test skill` fails until it matches the workspace version, and
  `scripts/release-check.sh` refuses a tag it does not match.
- The release workflow packages the skill with `python3 scripts/package-skill.py vX.Y.Z dist`, from the tag's commit
  in Git, not the working tree. It runs in the verify job and again before `gh release create`, which attaches
  `dist/cicada-skill-vX.Y.Z.tar.gz`. A skill that cannot be packaged (versions disagree, a file missing) stops the
  release. Run the script locally after tagging to see the archive.
- When the next change lands and sets the next version in `Cargo.toml`, set it in `skills/cicada/SKILL.md` too.

## 4. Changelog wording

The branch's `CHANGELOG.md` entry for 0.17.2 no longer links to README or guide sections. Once parts 1 and 2 exist,
the docs session may add links to them, and a line that the core guide gained the actor-design advice.

## 5. Skill routes to restore

`skills/cicada/SKILL.md` routes to files and headings that exist on main at the split. `examples/tests/skill.rs`
fails when a route stops resolving, so a heading the docs session renames needs the route changed in the same docs
merge (that file is code, outside the docs hook; tell the build session if the docs session should not edit it).

- It routes actor design to `docs/guide/core.md#design-an-actor` (section 3 and the "Actor design" row). If part 1
  lands under another heading, such as "Design an actor type", change both routes to it.
- It routes installing Cicada to `docs/how-to/install.md#choose-the-crates` and `README.md#using-a-release-in-another-project`.
  If part 2 adds a section on the skill, nothing in `SKILL.md` needs to point to it: the skill's section 7 tells the
  agent how to update itself.
- The test requires a route to every `.md` under `docs/guide`, `docs/how-to` and `docs/reference` (except
  `docs/how-to/releasing.md`, which is for Cicada's own releases), and forbids routes into `docs/maintaining/`. A new
  consumer page needs a row in `SKILL.md`.
