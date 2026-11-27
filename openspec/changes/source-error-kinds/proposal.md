# Proposal

## Why

The tray's failure row shows a fixed label by the kind of failure and never error text, as text from a server can echo a
token (`tray-menu-mockup`, code review round 6). It finds the kind by type in the error's source chain, but
`cicada-client` turns every failure of the HTTP turn source into a string (`turns.rs`: `client_error`, `unexpected`),
and `SourceError` carries only that string. So a real HTTP worker's failure nearly always shows "Connection failed",
where board Disconnected shows "Connection refused". The tray also cannot tell "Host not found", "TLS error" or "HTTP
401".

## What Changes

- `SourceError` gains a typed kind beside its message: refused, timed out, host not found, TLS, an HTTP status, or
  other. `cicada-client`'s HTTP turn source fills it from the reqwest error or the response status, never from text.
- The tray maps the kind to its labels (Connection refused, No answer, Host not found, TLS error, HTTP <status>,
  Connection failed), as `tray-menu-mockup`'s decision 3 names them.

To decide in the design: the shape (a field, or `SourceError` variants), and whether it is a breaking change for
implementers of `TurnSource` (the enum is public).

## Order

After `tray-menu-mockup`. Before `tray-settings-mockup`.

## Impact

Crates: cicada, cicada-client, cicada-tray.

`crates/cicada/src/runtime/source.rs` (`SourceError`), `crates/cicada-client/src/turns.rs`, `crates/cicada-tray` (the
label mapping). Public API of `cicada`; an ADR if the wire or a guarantee changes (expected: no). Docs handoff: the
HTTP guide's turn source text, if it names `SourceError`.
