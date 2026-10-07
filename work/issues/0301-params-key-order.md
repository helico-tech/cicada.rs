---
status: done
type: question
created: 2026-10-07
---

# 0301. Template parameters are served in sorted order, Kotlin serves them in template order

## Problem

`params` of `GET /actors/{path}` (`ActorView`) and `GET /types?uri=` (`TypeMatchView`) is a JSON object of the
address's template parameters. Kotlin builds it as a `LinkedHashMap` in template order
(`actor://account/{tenant}/{id}` gives `{"tenant": …, "id": …}`); the Rust core holds parameters in a `BTreeMap`
(`ActorSnapshot::params`, `ResolvedActorType::params`, `UriTemplate::matches`), so the Rust API serves them sorted
(`{"id": …, "tenant": …}`).

JSON objects are unordered (RFC 8259) and the console's zod schemas (`z.record`) accept either, so no known consumer
is affected. Every other object of the wire model has the Kotlin field order (checked against the console's
`generated.ts`), and the `messages` map of `TypeView` is sorted in both implementations.

## How to reproduce

`GET /actors/account/acme/7` on a Rust node: `"params":{"id":"7","tenant":"acme"}`; on a Kotlin node
`"params":{"tenant":"acme","id":"7"}`.

## Notes

To match byte for byte, the core would keep template order (e.g. a `Vec<(String, String)>` or an ordered map from
`UriTemplate::matches`) and the wire model would serialize it as an object in that order. That touches the runtime
results, which were being changed in parallel when this was found; decide whether exact key order is worth it.
