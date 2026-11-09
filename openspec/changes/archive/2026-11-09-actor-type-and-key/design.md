# Design

ADR 0013 holds the decision. This file holds what the ADR does not.

## Choices

- `ActorAddress` keeps the canonical text and a `TypeName`. The key is a slice of the text; decoded values are computed
  when asked for.
- `RefBinding` holds the type name and the ref's fields in declared order, read from serde's derive with the existing
  field probe. The serde bridge in `params.rs` stays; only the template around it goes.
- The store takes the type from the address. `create` loses its template argument, and `ActorRecord`, `Claim` and
  `ActorCreate` lose their template field.
- The resolver still reads `list_types` and keeps the contract cache. It finds a type by name instead of by matching.
- HTTP reads the two path segments raw and parses them as an address, so an encoded `:` stays part of a value. A key
  outside the allowed characters is `400`, where the old path parser was lenient.
- `prefix` stays a plain text prefix, so no store query changes for it.

## Mapping of existing templates

| Template | Type | Key |
| --- | --- | --- |
| `actor://orders/{id}` and the like | `orders` | `{id}` |
| `actor://batches/{batch}/items/{index}` | `batch-items` | `{batch}:{index}` |
| `actor://e2e/counter/{id}` and the like | `e2e.counter` | `{id}` |
| `actor://cicada/dead-letters` | `cicada.dead-letters` | `_` |
| `actor://maintenance/cleanup` | `maintenance.cleanup` | `_` |
| `actor://registries/devices` | `registries.devices` | `_` |
| `actor://payments`, `actor://pricing` | `payments`, `pricing` | `_` |

## Risks

- A key value such as `_` for a type with one field looks like a singleton's key. The type decides how a key is read,
  so nothing can mix them up.
- Proxies that mishandle `%2F` only matter for a key value that contains a `/`, which is rare now.
