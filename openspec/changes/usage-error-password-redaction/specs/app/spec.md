## ADDED Requirements

### Requirement: App-produced usage errors hide URL passwords

App-produced usage errors SHALL replace the password in recognized scheme://user:password@host text with ***,
including raw apostrophes, and SHALL process adjacent addresses separately. Redaction SHALL be idempotent and retain
exit code 2. This scoped scanner SHALL NOT promise arbitrary secret scrubbing, libpq/non-// forms, or credentials
truncated by whitespace, slash, question mark, hash, double quote or backtick. Ambiguous punctuation MAY be hidden.

#### Scenario: Apostrophe in a clap-rejected argument

- **WHEN** an App command rejects `postgres://u:S3'CRET@h/db` as an unexpected argument
- **THEN** its Usage error exits with code 2 and prints the address as `postgres://u:***@h/db`
- **AND** neither secret fragment appears elsewhere in the rendered error, and the application has not started

#### Scenario: Apostrophes in a settings error

- **WHEN** CICADA_LISTEN is `postgres://o'neil:S3'CRET@h/db` and settings reject it
- **THEN** the error retains the CICADA_LISTEN context and address `postgres://o'neil:***@h/db`
- **AND** no password fragment is printed and the application has not started

#### Scenario: Adjacent addresses

- **WHEN** a usage error contains `'a://u:p'q@h','b://v:r's@k'`
- **THEN** it prints `'a://u:***@h','b://v:***@k'`
- **WHEN** --types rejects `a://h,,b://u:secret@k` for an empty name
- **THEN** the error retains the first address and prints the second as `b://u:***@k`

#### Scenario: Credential characters and existing forms

- **WHEN** a recognized password contains sub-delimiters including apostrophe, percent escapes or Unicode
- **THEN** the whole password is replaced without decoding escapes or corrupting text
- **AND** an empty password is replaced, multiple at-signs use the last at-sign as today, and clear host/port/path context remains

#### Scenario: No password and repeated redaction

- **WHEN** diagnostic text contains ordinary text, user@host, host:port or bracketed IPv6 without a password
- **THEN** those forms remain unchanged
- **AND** applying redaction again to any already-redacted diagnostic produces the same output

#### Scenario: An incomplete authority

- **WHEN** a candidate authority has no at-sign
- **THEN** it is left unchanged because the scanner cannot distinguish a port from incomplete credentials
- **AND** the documented guarantee does not claim to hide such truncated input
