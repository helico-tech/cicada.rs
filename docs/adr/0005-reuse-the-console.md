# 0005. Reuse the React console unchanged

Date: 2026-10-07

## Status

Accepted

## Context

The Kotlin repo ships a React + TypeScript console (`actors-ui`) that talks only to the HTTP API and validates every
response with zod schemas generated from the Kotlin wire classes.

## Decision

`cicada-ui` contains a copy of that web app, builds it with Vite and embeds the output with rust-embed. `actor_ui()`
serves it next to the API with the same strict Content Security Policy and `config.json`. The generated wire types stay
as they are; a fixtures test records real responses of every endpoint of the Rust server and parses them with the
console's zod schemas, so the Rust API is checked against the console's expectations.

## Consequences

- The console is identical for both implementations.
- Building `cicada-ui` needs Node.js and npm, like the Kotlin build.
