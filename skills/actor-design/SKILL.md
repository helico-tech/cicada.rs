---
name: actor-design
description: Design a system with Cicada actors together with the user, one step at a time, before code is written, or review an existing actor design or implementation. Design: when the user wants to work out which actors exist, what each owns and decides, their keys and lifecycles, messages and replies, timeouts, failures and outside effects, or whether actors fit at all; it interviews the user through concrete stories, saves each confirmed step with Mermaid diagrams in docs/design/actors.md by default, and resumes from it. Review: when the user asks to review or critique an existing design or codebase; it reconstructs and draws the actors, messages and watches from evidence, checks them against the design guide and reports grounded findings with failure traces in docs/design/actors-review.md by default. It routes to the design guide of the Cicada source the project resolves. For writing, testing or running the code, use the cicada skill.
metadata:
  cicada-version: 0.19.4
---

# Actor design

This skill holds two methods: how to turn the user's problem into an actor design with them, and how to review an
actor design or implementation that already exists. The knowledge (what an actor owns, what a turn commits, which
patterns exist) lives in the design guide of the Cicada source the project resolves. Read it there; do not teach
Cicada from memory or from this file.

It designs and reviews; it does not build. Writing handlers, tests and manifests, running nodes and upgrading belong to
the `cicada` skill. Do not implement, fix, commit or deploy anything unless the user asks for that separately.

## Choose the mode

- **Design** a new system, or a new part of one: the nine-step interview of sections 2 to 6.
- **Review** when the user explicitly asks to review, critique or check an existing design document or codebase: the
  four phases of section 7 and the report of section 8. Do not take a review request through the design steps or
  their confirmations.
- **Unclear**, such as "look at our actors": ask one question (design something new, or review what exists?) while
  you already read the evidence the user supplied. Reading is all either mode does before the answer.
- **Both**, such as "review what we have, then design refunds": review the existing system first, then do only the
  design work the user asked for, in design mode and in its own document.
- Neither mode authorizes code fixes, dependency changes, commits or publication. A review's corrections are
  proposals.

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
While no source can be read, steps 1 and 2 can proceed; before step 3, tell the user which Cicada facts are unverified
and let them choose to resolve the source first or continue with those facts recorded as open questions.

**In a review** the statuses mean the same, with three differences. With `mixed`, ask which package and configuration
the review is for; reconstructing from the consumer's code and documents can go on, judgments that rest on Cicada's
semantics wait. With `none`, check the other selections, then review what is there without Cicada claims; the scope may
hold no actors at all. While no source can be read, reconstruct and check what does not depend on Cicada, and record
every judgment that does as unknown. With `self`, say that this skill reviews consumer systems.

## Source routes

Every route below is a path from the root of the resolved Cicada source, never from this skill's folder. Read the
route of the current design step or review area when it needs it.

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

A review checks seven areas (section 7). Each reads:

| Review area | Read in the source |
| --- | --- |
| Ownership | `docs/guide/design.md#choose-actors-and-identities`, `docs/guide/design.md#recognize-design-mistakes` |
| Identities and lifecycle | `docs/guide/design.md#choose-actors-and-identities`, `docs/guide/core.md#child-actors`, `docs/guide/core.md#watches`, `docs/guide/core.md#control-effects` |
| State and bounds | `docs/guide/design.md#choose-actors-and-identities`, `docs/guide/core.md#design-an-actor`, `docs/guide/core.md#how-deep-a-value-can-be` |
| Protocols | `docs/guide/design.md#design-protocols`, `docs/guide/core.md#message-persistence` |
| Coordination and time | `docs/guide/design.md#coordinate-work-and-time`, `docs/guide/actors.md#flows`, `docs/guide/core.md#timers` |
| Recovery | `docs/guide/design.md#recover-from-failure`, `docs/guide/core.md#retry`, `docs/guide/core.md#claim-limit` |
| Outside effects | `docs/guide/design.md#work-with-outside-systems`, `docs/guide/design.md#a-websocket-session` |

The guide and code of the resolved source win over this skill and over your memory. An older source may have no
`docs/guide/design.md`: then use its `docs/guide/core.md#design-an-actor`, its worked examples under `examples/` and
its public code, and record in the document or report that the design guide was not available at that version. If a heading is
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
  `needs review`) and the next questions.
- Create the document when the user has agreed to start a design and you write the first result; save every result
  as soon as it is drafted.
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
  with a design that uses no actors, or only a few, is a valid result; say why. When the user confirms that no actors
  are needed, or decides it within a delegation that covers step 1, the design ends there (section 6).
- **End each step with its written result and an updated drawing**, saved as `draft`, and show them to the user.
- **Wait for confirmation.** Move to the next step only after the user confirms this step's result, then mark it
  `confirmed`. Silence, a change of subject or a new question is not confirmation. Corrections are applied and shown
  again. Confirming a result accepts what it states; its labelled hypotheses and open questions stay open until the
  user settles them, so say which remain.
- **Delegation.** If the user explicitly lets you decide (all steps, some steps, or some kinds of choice), decide
  within that scope only, mark those results `delegated`, and write each choice with its reason and what stays
  uncertain under Decisions. Steps outside the scope keep the default: draft, show, wait. When the scope is unclear,
  ask. Never write `confirmed` for a delegated or drafted result. Delegation covers design choices only, not
  implementation, commits or deployment.
- **Go back when needed.** The user may return to an earlier step at any time (section 5).

## 4. Drawings

Draw in Mermaid inside the document. A drawing shows what the steps so far have established, and no more.

- **Steps 1 and 2:** a context drawing (step 1) and a timeline (step 2) of the domain: people, systems, facts and
  unknowns. They do not assert actor boundaries yet.
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
- **Revise.** When a requirement changes an earlier result, update that step and set it back to `draft` until the
  user confirms it again; within a delegated scope, decide the revised result yourself, mark it `delegated` and record
  the change under Decisions with its reason and what stays uncertain. Mark the later steps it affects `needs review`,
  keeping their text and the decisions it does not affect. Review those steps in order: with the user, or yourself
  where the delegation covers them.
- Update only the sections that changed.

## 6. Finish

The design is finished when step 9 is confirmed (or delegated), or when step 1 concludes that no actors are needed:
confirmed by the user, or decided by you within a delegation that covers step 1 and marked `delegated`. Record that
under Decisions, with the reason and what stays uncertain, and leave the later steps `not started`. Outside a
delegation, the no-actors ending waits for the user's confirmation. End with: the design and where it is saved, the
source it rests on and what was not verified, the open assumptions, and the next actions, such as building it with the
`cicada` skill. Do not start that work unless the user asks.

## 7. Review an existing system

A review has four phases, in order: 1. Scope and evidence, 2. Reconstruct, 3. Check, 4. Report. An explicit review
request authorizes all four; do not wait for confirmation after each, as design mode does. Show the reconstructed
system before any finding, invite corrections, and go on when the evidence suffices. Save the report as each phase
ends (section 8).

### Scope and evidence

- **Read first** the consumer's own instructions (AGENTS.md, CLAUDE.md, CONTRIBUTING.md or similar), any design
  document such as `docs/design/actors.md`, and a saved review report.
- **Scope.** Record the actors, stories and configuration (package, target, features) the review covers, and what it
  excludes. When the user names none, propose a scope from the evidence and say what you chose. Representative
  stories are not the whole codebase: never claim more coverage than you read.
- **Three identities, kept apart.** The consumer's code: the paths read, the revision and any uncommitted or
  unversioned changes, from read-only commands such as `git rev-parse HEAD` and `git status --short`; pasted material
  has no revision, so name it as pasted. The Cicada dependency: the resolver report (section 1). The installed skills:
  their versions and provenance. A newer skill does not make the project's Cicada newer.
- **Read only.** Reading files, read-only git commands and the bundled resolver (`cargo metadata --locked --offline`)
  are allowed. A review alone does not run the consumer's builds, tests, scripts or migrations, fetch or update
  dependencies, or write anything but its report. If the user separately asks you to run a test, it has its normal
  effects; report exactly what ran and what it showed.
- Never copy credentials, tokens or private URLs with secrets into the report.

### Reconstruct

Draw what the evidence shows before you judge it, in Mermaid as in section 4:

- **An actor map**: actor types and their keys, the messages between them, watches and parent/child relations as
  distinct kinds of line, outside systems, and host code that sends messages or holds live handles. Keep host calls
  and outside I/O apart from messages that a turn commits.
- **A sequence diagram per selected story**: the success path and its failure branches (timeout, repeat, reorder,
  cancellation, a failed dependency), marking what one turn commits together and what happens outside a turn.
- **A state diagram** for each actor whose states or lives matter to the scope.
- **Mark inferred and unknown edges** (a dashed line or a `?` in the label) and explain the marks in a legend under
  the drawing. A partial map is fine when the evidence is partial; say what is missing.
- For a tiny scope, or one without actors, draw its real context (functions, tables, outside systems). Never invent
  actors or watches to fill a drawing.

Tag every claim with where it comes from:

| Tag | Means |
| --- | --- |
| observed | Read in code or tests. Cite the file, the symbol and the line. |
| documented | Stated in a design or document. Cite the file and heading; for pasted text, its section and a short quote. |
| inferred | Your reasoning from observed or documented facts; say from which. |
| unknown | Not in the evidence. It becomes an open question when it matters. |

Say which tests you read and which ran; reading a test is not running it. Where a document and the code disagree,
show both with their evidence; do not pick one silently. A missing document does not prove that a behaviour is
missing, and a proposal or Cicada's latest `main` proves nothing about what the consumer runs.

### Check

Check seven areas against the resolved guide (their routes are under Source routes): ownership, identities and
lifecycle, state and bounds, protocols, coordination and time, recovery, outside effects. Give each one verdict with
its evidence or reason: `supported`, `finding` (with the ids), `unknown` or `not applicable`. The yardstick is the
consumer's requirements and Cicada's actual guarantees, not one preferred decomposition. Do not import another actor
framework's semantics, and treat proposed or future Cicada features as unavailable.

- **A finding** names a requirement the system violates, or a failure risk the evidence supports, with its
  preconditions and a short counterexample trace: the initial state, the messages and failures in the order that
  breaks it, and the bad outcome. It has an id, a priority, evidence, the consequence, the smallest useful correction
  and a test or trace that would check it (section 8). One cause is one finding, however many places show it.
- **Priority is impact, not confidence.** `high`: credible loss of data or isolation, an irreversible duplicate outside
  effect, or accepted work stuck without recovery. `medium`: wrong but recoverable behaviour, or unbounded resource
  growth. `low`: a bounded maintainability issue or protocol ambiguity. Explain the impact in the reviewed scope.
- **Open questions.** When a requirement or evidence is missing, write the question and its conditional risk ("if a
  reply can arrive after cancel, then ..."), not a defect. Ask at most three focused questions per turn, only where
  the answer changes a conclusion, and keep checking what does not depend on them. An unanswered question stays
  unknown; silence is not approval. A delegation lets you make the choices it names; it never lets you invent facts
  about existing code.
- **Accepted tradeoffs.** A documented tradeoff that meets the requirements is recorded as accepted, not turned into a
  finding because another decomposition is possible. A claimed tradeoff that breaks a stated invariant is still a
  finding, with its evidence.
- **No findings** is a valid result. Never fill a quota, and never present a clean scope as proof that the whole
  system is correct.

### Report

Complete the report (section 8) and end with a short summary: where it is saved, the findings by priority, the open
questions, what was not checked, and the next actions. Do not implement a correction unless the user asks separately;
building belongs to the `cicada` skill.

## 8. The review report

- **Where.** `docs/design/actors-review.md` from the repository root, unless the user names a path or the
  repository's own rules put such documents elsewhere. It is its own file: never write the review into the design
  document or the code.
- **Look before you write.** Resume an existing report only when its Scope and sources and Progress name the same
  system and review scope; a changed revision is fine. A report of another system or scope, unrelated content, or an
  identity you cannot tell is a collision: ask for another path or for clarification, and change nothing.
- If there is no writable repository, or the user does not want files written, show the same report in the chat and
  say that it was not saved. Never commit the report yourself.
- **Shape.** A new report has these sections, in this order. An existing report keeps its shape and the user's
  content; add what is missing.

```markdown
# Actor review: <system and scope>

## Scope and sources

- **Requested scope:** <actors, stories, configuration>; **excluded:** <what is not reviewed>
- **Consumer code:** <paths read, revision, local changes; or "pasted on <date>">
- **Cicada source:** <version, Cargo source and revision as the resolver reports them>
- **Skills:** actor-design <version>, cicada <version>; <match, difference, or not comparable>

## Progress

Phase: <Scope and evidence, Reconstruct, Check, Report or done>
Pending questions: <at most three>
Evidence snapshot: <consumer revision and local changes, Cicada source, date>

## Reconstructed system

<Actor map, sequences per story, state diagrams, each with a legend for inferred and unknown edges.>

## Coverage

| Area | Verdict | Evidence or reason |
| --- | --- | --- |
| Ownership | <supported, finding R-001, unknown, not applicable> | <evidence> |

## Findings

### R-001 <short title>

- **Priority:** <high, medium, low>, because <impact>. **Status:** <open, stale, accepted, deferred, resolved,
  withdrawn, merged into R-00n>
- **Evidence:** <tagged claims with locations>
- **Trace:** <initial state; the messages and failures in order; the bad outcome>
- **Consequence:** <what goes wrong for whom>
- **Correction:** <the smallest useful change>
- **Check:** <a test or trace that would show it fixed>
- **History:** <earlier traces, snapshots and dispositions>

## Accepted tradeoffs

## Open questions

## Checks and limits

<What was read, what ran (nothing, by default), tests read but not run, Cicada facts not verified, what is out of scope.>

## Next actions
```

- **Ids.** Findings are numbered R-001, R-002 and on. Never renumber. The same cause at the same location keeps its
  id; a new finding takes the next unused number. A withdrawn or merged id stays listed with that status and is never
  reused.
- **Resume.** Read the report first. Then check again the consumer's revision and local changes, the resolver report
  and the scope, and record the new evidence snapshot. Observations and findings that rest on changed evidence become
  `stale` until you check them again; if you cannot tell what a change affects, mark the whole review stale in
  Progress. Continue at the recorded phase and pending questions. Keep the ids, earlier traces, the user's notes and
  dispositions.
- **Dispositions.** `resolved` needs new evidence that the cause is gone; cite it and keep the earlier trace under
  History. `accepted` (the user takes the risk) and `deferred` are not resolved. A new run does not erase earlier
  limits or imply that tests ran.
- Update only the sections that changed.

## 9. Install and update

This skill ships with its sibling `cicada`, in the same version: in the Claude Code plugin `cicada@cicada` (used as
`/cicada:actor-design`), and in the release archive `cicada-skill-vX.Y.Z.tar.gz`, which holds the folders `cicada`
and `actor-design`. Install and update both folders together and keep them side by side; this skill needs the
sibling's resolver. The plugin is updated through `/plugin`. For an archive, unpack it into a staging folder, compare
both folders with the installed copies, and replace them only after the user has seen local changes. Updating a skill
never changes the project's Cicada dependency.
