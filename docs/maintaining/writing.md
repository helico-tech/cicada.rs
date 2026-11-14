# Documentation writing standard

Make the next useful action easy to find. Optimize for a reader with a task and limited attention, including agents
that retrieve one section at a time. This standard was researched on 2026-11-14.

## Structure

Give each page one job. [Diátaxis](https://diataxis.fr/) separates learning, task instructions, reference and explanation.
Cicada applies that distinction within its existing layer structure:

| Reader need | Home | Shape |
| --- | --- | --- |
| First success | Core starter | Prerequisites, complete actor, commands, observable result |
| Complete a task | Layer guide / how-to | Outcome first, steps and the decisions needed now |
| Look up exact details | Architecture, rustdoc, generated reference | Stable names, precise rules, links |
| Understand a decision | Architecture / ADR | Model, rationale, tradeoffs |

Start at [docs/README.md](../README.md). Keep README an introduction and a path into the docs.
Do not make readers traverse every layer to complete one task. Move long migration history out of the daily runbook.
Accepted ADRs remain historical records; do not rewrite them to satisfy length targets.

## Prose

[Google’s technical-writing guidance](https://developers.google.com/tech-writing/one/documents) recommends defining the
scope/audience, summarizing essentials first, and organizing around the reader’s needs.

- Lead with what the reader will accomplish. Put prerequisites before the action that needs them.
- Use short paragraphs and concrete verbs. Define a domain term once and use the same spelling everywhere.
- Use numbered lists for procedures and small tables for choices or parallel facts.
- Put failure consequences next to the operation: retry can repeat work; delete removes descendants.
- Link to the authority for complete edge cases. Avoid repeating the same contract in several guides.
- Split a page when it acquires a second task, not to satisfy a word-count target.

## Examples

[Google’s sample guidance](https://developers.google.com/style/code-samples) calls for an introduction, consistent code
style and explicit omissions. Cicada uses these rules:

- Keep one complete starter actor. Other sections show only the new operation or link to a complete tested example.
- Say what the code does and what result to expect. Name the source file for a worked example.
- Use real type and method names. Avoid placeholders, fake JSON and unfinished loops in executable instructions.
- Put setup required only by rustdoc in an HTML `doctest` comment so GitHub readers do not see scaffolding.
- Compile every Rust block. Use `no_run` only when a database/server or other external resource is needed.
- Keep important executable coverage in examples/tests; shortening prose must not remove the only behavioral test.

## Visuals

Use Mermaid to explain message paths, ownership or replay when a picture removes the need for more prose.
Label the actual messages and commit boundaries, and give each diagram a short caption with its ordering limits.
Use exact message variant names on arrows; when an actor forwards a wrapped message, name both the envelope and
the target's message. Mark external calls and return values separately from queued messages.
Keep diagrams narrow enough for a guide page; parse and render them with the [visual tooling](../assets/README.md)
before committing. Screenshots show the real application against isolated sample data, with instructions to refresh
when the interface changes. Never use an invented UI as evidence of current behavior.

## Retrieval by agents

[Anthropic’s context-engineering guidance](https://www.anthropic.com/research/effective-context-engineering-for-ai-agents)
recommends focused context and retrieval as needed. Our application is a short [agent map](../agents.md) pointing at
version-matched sources, rather than a duplicate agent manual.

Use descriptive Markdown headings, relative links, exact symbols and a clear authority order. Preserve useful anchors
when shortening a guide, and update links when a topic moves. Distinguish released behavior from proposals.
Do not add a second generated bundle that can drift from the guides.

## Review

A change is ready when a reader can find a task, run the example, recognize success and find recovery guidance.
Check links, compile snippets, inspect source for changed facts, and read the rendered structure as a reader.
Passing checks proves structure and compilation; it does not prove that the prose teaches well.
