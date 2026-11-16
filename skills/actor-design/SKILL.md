---
name: actor-design
description: Design a system with Cicada actors together with the user, one step at a time, before code is written. Use it when the user wants to work out which actors exist, what each owns and decides, their keys and lifecycles, the messages and replies between them, timeouts, failures and outside effects, or whether actors fit the problem at all. It interviews the user through concrete stories, writes each step's result with Mermaid diagrams into a design document in the repository (docs/design/actors.md by default), waits for confirmation before it moves on, and resumes from that document. It routes to the design guide of the Cicada source the project resolves. For writing, testing or running the code, use the cicada skill.
metadata:
  cicada-version: 0.19.2
---

# Actor design

This skill holds a method: how to turn the user's problem into an actor design with them. The knowledge (what an
actor owns, what a turn commits, which patterns exist) lives in the design guide of the Cicada source the project
resolves. Read it there; do not teach Cicada from memory or from this file.

It designs; it does not build. Writing handlers, tests and manifests, running nodes and upgrading belong to the
`cicada` skill. Reviewing an existing system against a design is not part of this skill. Do not implement, commit or
deploy anything unless the user asks for that separately.

## 1. Find the project's Cicada source

This skill uses the resolver of its sibling skill `cicada`, installed next to it: the folder `cicada` beside the
folder that holds this SKILL.md, in a Claude Code plugin and in an unpacked release archive alike. Run it from the
project folder, with the actual path quoted for the shell:

```sh
python3 "/path/to/skills/cicada/scripts/cicada_source.py"     # host target, default features
python3 "/path/to/skills/cicada/scripts/cicada_source.py" --manifest-path crates/api/Cargo.toml \
    --target x86_64-unknown-linux-gnu --no-default-features --features postgres
```

It is read-only and offline (`cargo metadata --locked --offline` for one target and feature selection), never fetches,
installs or changes the project, and prints one JSON report. Read the sibling's SKILL.md, section 1, for the meaning of
every field. Act on `status` as it says:

| `status` | For a design |
| --- | --- |
| `ok` | One source. Read the routes below beneath its group's `root`. |
| `mixed` | Several versions or sources. Show the groups and ask which package and configuration the design is for. Exploring the problem (steps 1 and 2) can go on; Cicada-specific decisions wait. |
| `no-docs` | The package is resolved but its tree has no docs. Read that package's source at its `manifest_path`, or its rustdoc, and record that the design guide was not available. Cicada is not absent. |
| `none` | No Cicada crate in this selection. Check another `--manifest-path`, `--target` or `--features` first. For a design ahead of a new integration, the cicada skill's section on a new integration says how to choose a release; until one is chosen, record the source as not chosen. |
| `self` | The project is the Cicada repository itself, not a consumer. Say so; this skill designs consumer systems. |
| `unavailable` | Read `reason` and `diagnostic`, then use the sibling's manual path (`Cargo.lock`, manifests, verified checkouts only). A `hint` to run `cargo fetch --locked` downloads, so it needs the user's consent; never `cargo update`. |

**Two skills, one report.** The report's `skill_version`, `skill_source`, `version_relation` and `revision_match`
describe the sibling `cicada` skill, not this one. Compare this skill's `metadata.cicada-version` with
`skill_version`, and, when this folder holds a `SOURCE.json`, its `commit` with the sibling's `skill_source`. Say when
they differ or cannot be compared; never present them as a match without that evidence. The project's resolved source
is the authority over both skills either way.

**Missing sibling.** If the sibling folder or its resolver is missing, or its version differs, say so. Use the
sibling's manual path where you can read it, otherwise read `Cargo.lock` and the manifests yourself: the `name`,
`version` and `source` of each Cicada crate, and the source only where it verifiably is (a `path` dependency's folder,
a checkout Cargo reports). Never install the sibling, change dependencies, or fall back to Cicada's latest `main`, a
newer release or a proposal. Generic exploration of the problem can go on; record the limitation in the document.

## Source routes

Every route below is a path from the root of the resolved Cicada source, never from this skill's folder. Read the
route of the current step when the step needs it.

| Need | Read in the source |
| --- | --- |
| The method's steps and what each step writes down | `docs/guide/design.md#from-problem-to-design` |
| Owners, keys, creation and children | `docs/guide/design.md#choose-actors-and-identities`, `docs/guide/core.md#design-an-actor`, `docs/guide/core.md#child-actors`, `docs/guide/core.md#watches` |
| Messages, replies, acknowledgements, state tables | `docs/guide/design.md#design-protocols` |
| Coordination, sagas, flows, deadlines | `docs/guide/design.md#coordinate-work-and-time`, `docs/guide/actors.md#flows`, `docs/guide/core.md#timers` |
| Failure and recovery | `docs/guide/design.md#recover-from-failure`, `docs/guide/core.md#retry`, `docs/guide/core.md#control-effects` |
| Outside effects, live handles held by a host | `docs/guide/design.md#work-with-outside-systems`, `docs/guide/design.md#a-websocket-session` |
| Choosing a pattern | `docs/guide/design.md#choose-a-pattern`, `docs/guide/actors.md` |
| Checking and simplifying | `docs/guide/design.md#recognize-design-mistakes` |
| What a turn guarantees | `docs/architecture.md#guarantees`, `docs/architecture.md#known-limitations` |

The guide and code of the resolved source win over this skill and over your memory. An older source may have no
`docs/guide/design.md`: then use its `docs/guide/core.md#design-an-actor`, its worked examples under `examples/` and
its public code, and record in the document that the design guide was not available at that version. If a heading is
missing, search that source's `docs/` for it. Never fetch a newer guide or treat a proposal as shipped.

## 2. The design document

The result lives in one Markdown file in the user's repository, so that it renders on GitHub and in editors and a
later session can resume it.

- **Where.** `docs/design/actors.md` from the repository root, unless the user names a path or the repository's own
  rules put design documents elsewhere. Look at the destination before you write. If it holds another design, ask for
  a new name; never replace unrelated work. If it holds this design, resume it (section 5).
- **Shape.** A new document starts from `assets/design-template.md` in this skill's folder: Source and scope,
  Progress, one section per step with the step names of the guide, Decisions and Open questions. An existing document
  keeps its shape; add what is missing, do not rewrite it into the template.
- **Source and scope** records the resolved source (version, Cargo source, revision, the target and features examined),
  the skill versions, and what could not be verified. Never copy credentials, tokens or private URLs with secrets.
- **Progress** records the current step, each step's status (`not started`, `draft`, `confirmed`, `delegated`,
  `needs review`) and the next question.
- If there is no writable repository, or the user does not want files written, show the same document in the chat and
  say that it was not saved. Never commit the document yourself.

## 3. Work one step at a time

Follow the nine steps of the guide's table under From problem to design, in its order: 1. The promise, 2. Who and
what, 3. Who owns each rule, 4. Identity and life, 5. Responsibility cards, 6. Protocols, 7. Coordination and time,
8. The outside, 9. Check and simplify. Each step asks the guide's questions and writes down what the guide's table
says.

- **Start from stories.** Ask for one or two concrete stories, each with a way it goes wrong: an order whose payment
  times out, a device that reconnects after its command was sent. Use these stories in every later step.
- **Ask at most three questions at a time**, focused on the current step and grounded in a story. Do not ask about
  later steps yet.
- **Use the user's words.** Name actors, messages and states in the user's vocabulary, not in Cicada's or yours.
- **Do not invent requirements.** A guess is written as a hypothesis or an open question and labelled so. An actor is
  added only for a rule that needs an owner over time.
- **Step 1 asks whether actors fit.** A function, a table with a transaction or a read model may be simpler. Ending
  with a design that uses no actors, or only a few, is a valid result; say why.
- **End each step with its written result and an updated drawing**, saved as `draft`, and show them to the user.
- **Wait for confirmation.** Move to the next step only after the user confirms this step's result, then mark it
  `confirmed`. Silence, a change of subject or a new question is not confirmation. Corrections are applied and shown
  again.
- **Delegation.** If the user explicitly lets you decide (all steps, some steps, or some kinds of choice), decide
  within that scope only, mark those results `delegated`, and write each choice with its reason and what stays
  uncertain under Decisions. Never write `confirmed` for a delegated or drafted result. Delegation covers design
  choices only, not implementation, commits or deployment.
- **Go back when needed.** The user may return to an earlier step at any time (section 5).

## 4. Drawings

Draw in Mermaid inside the document. A drawing shows what the steps so far have established, and no more.

- **Steps 1 and 2:** a context or timeline drawing of the domain: people, systems, facts and unknowns. It does not
  assert actor boundaries yet.
- **From step 3 on:** an actor map once owners emerge: actors, the messages between them (named as in the user's
  words), watches and parent/child relations as distinct kinds of line, outside systems, and the host where one holds
  a live handle.
- **Steps 4 to 6:** a state diagram for each actor with a lifecycle worth drawing; a message table and a state table.
- **By step 7:** a sequence diagram per chosen story with the success path and branches for timeout, repeat,
  reorder, cancellation and a failed dependency, or a line saying why a branch does not apply. Show which work is a
  host call outside a turn and which state and sends commit together in one turn.
- Do not add actors to fill a drawing. Keep each drawing small; split it rather than crowd it.

## 5. Resume and revise

- **Resume.** Read the document first. Run the resolver again and compare the source with Source and scope; say when
  it changed. Continue at the recorded step and next question; do not restart the interview, and do not mark a draft
  confirmed because it was saved.
- **Revise.** When a requirement changes an earlier result, update that step and mark the later steps it affects
  `needs review`, keeping their text and the decisions it does not affect. Review those steps with the user in order.
- Update only the sections that changed.

## 6. Finish

The design is finished when step 9 is confirmed (or delegated). End with: the design and where it is saved, the
source it rests on and what was not verified, the open assumptions, and the next actions, such as building it with the
`cicada` skill. Do not start that work unless the user asks.

## 7. Install and update

This skill ships with its sibling `cicada`, in the same version: in the Claude Code plugin `cicada@cicada` (used as
`/cicada:actor-design`), and in the release archive `cicada-skill-vX.Y.Z.tar.gz`, which holds the folders `cicada`
and `actor-design`. Install and update both folders together and keep them side by side; this skill needs the
sibling's resolver. The plugin is updated through `/plugin`. For an archive, unpack it into a staging folder, compare
both folders with the installed copies, and replace them only after the user has seen local changes. Updating a skill
never changes the project's Cicada dependency.
