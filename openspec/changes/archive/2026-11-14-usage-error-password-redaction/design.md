# Design

## Boundary and scanner

Keep command::hide_passwords as a text scanner shared by clap error rendering and AppError::usage. Raw apostrophes
are legal inside credentials and clap does not escape them. Remove apostrophe from the authority terminators; keep
whitespace, slash, question mark, hash, double quote and backtick as hard boundaries. Preserve the first-colon to
last-at-sign rule within a candidate authority, replacing only the intervening bytes with ***. A closing clap quote
after the host can stay in the scanned span and is copied unchanged with the host.

Before redaction, detect a following :// at the current hard boundary: if the bounded prefix ends in a colon and the
remainder begins with //, walk backward from that colon over ASCII letters, digits, plus, minus and dot. End the
current candidate at the start of that scheme-like run, leaving it for the outer scanner. This is conservative text
recognition, not full URI validation; do not require a successful URL parse before hiding credentials. The outer loop
has already consumed the current ://, so even a zero-length candidate makes progress. Slice only at character-safe
boundaries; the backward run contains only ASCII bytes.

This preserves separate processing of adjacent quoted or comma-separated addresses. For example,
`'a://u:p'q@h','b://v:r's@k'` becomes `'a://u:***@h','b://v:***@k'`, and an empty-name error for
`a://h,,b://u:secret@k` retains both addresses with the second password hidden. Do not consume the next scheme's colon
as a port and then lose its ://. Ambiguous punctuation can be hidden too; credentials take precedence over preserving
every punctuation character. Keep ordinary host, port, path and diagnostic context where the boundary is clear.

No-password forms (user@host, host:port, bracketed IPv6 plus port) stay unchanged. Empty passwords become ***. Multiple
at-signs retain the existing last-at-sign rule. Percent escapes are copied, not decoded; Unicode in credentials is
hidden without corrupting text. A truncated span without an at-sign stays unchanged because a password cannot be
distinguished from a port there. Do not claim coverage for malformed or delimiter-truncated credentials.

The scope remains usage errors for scheme://user:password@host-style text. Raw slash, question mark, hash, double
quote, backtick and whitespace in passwords remain outside this scanner's boundary. Non-// addresses and libpq
connection strings are unchanged. Do not alter Target Debug, other error channels or arbitrary externally constructed
AppError::Usage strings. Describe the guarantee for the App-produced usage path, not a general secret scrubber.

Parsing each address with url still requires finding its end and risks displaying malformed data on parse failure.
Pairing clap quotes is ambiguous with a raw apostrophe. Hiding everything up to whitespace loses useful hosts/ports.
The two scanner changes solve the observed cases without a dependency, parser overhaul or new public API.

## Proofs

Extend the existing helper fixtures with apostrophes in user/password, a trailing apostrophe before @, sub-delimiters,
percent-encoding and Unicode, empty/multiple-@ credentials, neighbouring quoted URLs, the comma-separated --types
case, no-password/IPv6/port forms and ordinary text. Assert precise useful output for clear boundaries. For every
fixture, applying the helper twice must equal applying it once: parse already redacts, then AppError::usage does so
again. Do not pin unsupported leaks as desired behavior or assert internals of the scanner.

Extend the actual App tests, not only a copied clap tree: serve with an unexpected URL containing S3'CRET must return
Usage/exit2 with neither fragment left; CICADA_LISTEN with an apostrophe in both user and password must pass through
settings/AppError::usage without leaking; --types with a passwordless URL followed by a secret-bearing URL must hide
the second password. Preserve the existing no-start assertions and useful error prefixes. Keep existing tests and
test both hosted and settings paths as their current harness allows. Give settings tests a valid store setting through
TempDb::env so resolution reaches the intended failure; no network/database is opened for these failures.

## Artifacts and scope

Add the app spec's usage-redaction requirement. Update hide_passwords and AppError::Usage doc comments to distinguish
apostrophes from unsupported delimiters, and add a scoped clause to the app guide. No CLI command/flag/help changes,
so no generated CLI rewrite is expected; its existing test must still pass. Add changelog and the patch version chosen
when built, after the preceding releases, rather than the adviser's historical 0.16.18 suggestion.

No ADR: this fixes existing diagnostic behavior without a wire, schema, API or G/S guarantee change. Three tasks
plus finishing fit. The owner explicitly requested this and postgres-close-cancellation despite the ready-queue cap;
design them in order, leaving the preceding changes and the builder's worktree alone.
