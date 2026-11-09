---
status: wontfix
type: debt
created: 2026-10-07
---

# 0300. The Rust client cannot address instances with a `.` or `..` segment

## Problem

`ActorAddress::to_http_path` writes the segments `.` and `..` as `%2E` and `%2E%2E`, so that clients and proxies do
not remove them as RFC 3986 dot segments. That works for the Kotlin client (Ktor), but the Rust client is built on
reqwest, whose URL parser (the `url` crate, WHATWG URL standard) also treats `%2e`, `.%2e`, `%2e.` and `%2e%2e`
(any case) as dot segments and removes them. `/actors/account/%2E%2E/1` would become `/actors/1` and address
another instance.

`cicada-client` therefore refuses such addresses (and message ids `.` and `..`) with `ClientError::Invalid` instead of
misaddressing them. The server serves them fine (`cicada-axum` test `dot_segments_are_addressed_as_themselves`), and
the in-process `ActorSystem` API is not affected. The Kotlin test `dot segments are addressed as themselves, not
normalised away` of `ActorClientTest` is ported as `dot_segments_are_refused_rather_than_misaddressed`.

## How to reproduce

```rust
let actors = ActorClient::new(base);
actors.create(&Account { tenant: "..".into(), id: 1 }, &state).await  // Err(ClientError::Invalid(..))
```

## Notes

Options:

- Keep the refusal (current). Costs nothing; such addresses are rare. Callers can use the in-process API.
- Send requests without the `url` crate's normalisation (hyper directly, or a reqwest feature if one appears). Keeps
  the contract; a second HTTP stack in the client.
- Add an alternative escape to the HTTP contract that URL parsers leave alone. A contract change: needs an ADR and the
  same change in the Kotlin implementation.

- 2026-10-07: decided to keep the refusal. The server and the in-process API serve these addresses; only the Rust HTTP client cannot, and it fails loudly instead of misaddressing. A second HTTP stack or a contract change costs more than addresses whose segment is literally `.` or `..` are worth. Reopen if a user needs them over HTTP from Rust.
