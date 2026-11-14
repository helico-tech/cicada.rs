# Proposal

## Why

A usage error of the command line hides the password of an address it quotes (`hide_passwords` in
`crates/cicada-app/src/command.rs`, called from `usage` there and from `AppError::usage` in `crates/cicada-app/src/lib.rs`).
The scan stops at a quote, because clap quotes what it cannot use with `'`. A valid URL may hold a raw `'` in its
password (RFC 3986 sub-delims), so `postgres://u:can'ary@h/db` given where clap refuses it is printed with its password.
The helper's doc comment lists this as left alone. Found while building `app-worker-over-http`, whose `Target` Debug
now parses the URL instead (`redacted_url` in `crates/cicada-app/src/config.rs`).

An apostrophe in the user name also cuts the scan short. Adjacent addresses expose another boundary problem:
`--types 'a://h,,b://u:secret@k'` can print the second password because the first scan consumes its scheme before
stopping at its slash. Both cases belong to the same usage-error scanner fix.

## What Changes

- A usage error never prints the password of an address it quotes, also when the password holds a raw `'`. When the
  end of an address cannot be told for certain, hide more, not less.
- Regressions for a password with `'` in a clap error and in an `AppError::usage` message, next to the existing cases.
- Recognize a following address before consuming its scheme, so adjacent addresses are each redacted; preserve
  idempotence because command parsing and AppError construction can both redact the same error.
- The helper's doc comment says what it hides.

## Out of scope

Characters that are not legal raw in a URL password (`/`, `#`, `?`, `"`, a space), the address without `//` and the
libpq form, which the helper leaves alone today; other output than usage errors; `Target`'s Debug, already fixed.

## Order

After `cicada-agent-skill`. Before `store-close-cancellation`.

## Impact

`crates/cicada-app` (`command.rs`, `lib.rs`) and its tests. No public API, wire format, schema or guarantee changes.
