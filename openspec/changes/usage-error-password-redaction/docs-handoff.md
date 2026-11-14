# Docs handoff for usage-error-password-redaction

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone.

What the code branch holds: the scanner fix in `crates/cicada-app/src/command.rs` (`hide_passwords`), the doc comments
of `hide_passwords` and `AppError::Usage`, the tests in `command.rs` and `crates/cicada-app/tests/app.rs`, the
changelog entry and version 0.17.3. No command, flag or help text changed, so `docs/reference/cli.md` is unchanged.

## 1. Passwords in usage errors, in the app guide

**Where.** `docs/guide/app.md`, the operator-commands section, right after the sentence "Missing items and API failures
exit 1. Usage errors exit 2." (today around line 140). A sentence near the `--store` row of the settings table would do
as well; one place is enough.

**What users must learn.** A usage error quotes what the user gave, and that may be an address with a password. The
binary hides the password of a `scheme://user:password@host` address in its usage errors, also when the password holds
a raw `'` or another character that is legal in a URL password, and it hides each of several addresses in one value on
its own. It is scoped: it is not a general secret scrubber. A password with a raw `/`, `?`, `#`, `"`, `` ` `` or a
space (which must be percent-encoded in a URL anyway), an address without `//`, the libpq form and other output than
usage errors are not covered. Do not claim that all error text is scrubbed.

**Agreed text.**

> A usage error hides the password of an address it quotes: `postgres://u:pw@h/db` is printed as
> `postgres://u:***@h/db`, also when the password holds a raw `'`, and each address of a list on its own. It covers
> only `scheme://user:password@host` in usage errors: percent-encode `/`, `?`, `#`, `"`, `` ` `` and spaces in a
> password, as a URL needs anyway.
