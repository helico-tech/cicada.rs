# Design

## Boundary and representation

Failed already wraps an accepted payload of at most 64 levels into a notice of at most 65. Typed watchers and remote
workers can read it; storing the notice in their own state must still meet 64. Keep core construction, decoding and
limits unchanged. Add boundary proofs and correct the debouncer witness comment. Raising the limit or granting a
special type an exception would weaken ADR 0029 and is unnecessary.

A dead-letter state adds three containers around its payload. Record payloads of depth <=61 in the existing payload
field; record deeper payloads as compact JSON text in a new payloadJson string field. Exactly one field is present.
Do not use an in-band marker: any JSON value, including null, a JSON-looking string or a marker object, can be an old
raw payload. Keeping only a message reference would lose data with outcome retention. Always encoding would hide
ordinary letters from operators without benefit. No new type name, child storage, core helper or automatic repair.

Change DeadLetter.payload to public LetterPayload::{Value(Value), Json(String)}, re-exported from cicada-actors, with
value(&self) -> Result<Value, serde_json::Error>. Value clones; Json parses on demand with serde_json's normal recursion
limit. Do not parse JSON text while decoding state: one damaged encoded letter must not prevent Discard or other
letters from working. Serialization preserves the selected form, including an explicitly encoded shallow value.

Use a private wire struct/conversion and an honest JSON Schema requiring exactly one of payload/payloadJson, with
payloadJson constrained to string and payload accepting every value. Presence tracking must distinguish payload:null
from an absent field; plain Option<Value> is insufficient. Both fields, neither field, or payloadJson:null fail typed
decoding and the HTTP schema check (422, not 400). Unknown unrelated fields retain existing lenient decoding.
Avoid serde flatten/untagged buffering over the deep payload. A custom schema/conversion is allowed if derives cannot
express the rule; generated-schema acceptance and rejection tests must prove parity with serde.

record uses a private bounded depth walk, stopping once a container at level 62 proves encoding is needed. Use an
explicit stack of iterators so auxiliary storage is bounded by nesting, not by the width of the payload. Serialization
still has serde's ordinary bounds; the guarantee covers accepted payloads, not arbitrary hand-built unbounded Values.

## Redrive and numeric fidelity

Redrive calls value before send_raw. Parse errors set redriveError to "The kept payload cannot be read: ...", send
nothing and retain the original text. Successfully decoded values pass through send_raw's existing depth/NUL checks;
those errors are recorded as today. Store errors still fail the turn; an absent destination still rejects the whole
commit. Keep the second-redrive guard, metadata, capacity and Discard semantics. Encoded JSON is not a validation bypass.

Enable serde_json's float_roundtrip feature in the workspace dependency, inherited by every crate. The additional
Value -> text -> Value conversion preserves semantic Value equality, exact integer values and finite f64 bits,
including negative zero. Original JSON spelling/order is not promised. This fixes text parsing, not database numeric
normalization: PostgreSQL JSONB may normalize signed zero and numeric representation before a Failed reaches the actor
or after a redrive is sent. Compare to the captured Failed payload for encoding tests, and test/document backend
normalization separately. Do not promise preservation of bits lost before capture or by subsequent JSONB storage.

Use the regression value 1.0715660391465826e-75 and its negative, minimum subnormal, maximum finite f64, -0.0, and
i64/u64 limits. Check f64 bits, since Value equality cannot distinguish signed zeros. Include an isolated consumer
feature check so workspace test feature unification cannot conceal a missing feature in published dependencies.

## Legacy data and upgrade

The state schema/hash and public Rust field type change. ADR 0043 records a breaking release (next compatible
breaking version, expected 0.17.0), no SQL migration, and no change to Failed or other watcher contracts. Start-time
contract checks and remote claim checks reject mismatches; they do NOT evict already-running old local hosts.
Every old host must stop before replacing the registration. Old typed readers cannot read payloadJson-only letters.

Document and exercise a maintenance procedure for independent, childless dead-letter actors: quiesce watched work
and external senders first, drain their turns and then all dead-letter mailboxes; stop every local/remote host and
automatic bootstrapper. Keep a database backup and export ALL dead-letter actors' states, addresses, pause settings
and watches (both as target and watcher). Verify no pending/leased work remains. Do not delete on an incomplete export.
Use a new handler-free management server, delete all instances of this type, remove its registration, register the
new contract, recreate at the same addresses from saved states, restore pause settings/watches, verify, then resume.
Versions/creation times/epochs are new; saved letter metadata and redrivenAs stay. Do not reuse cached turn leases.

Deletion removes descendants and emits Stopped to address watchers. The simple recipe requires no parent/children
and no incoming watches; detect these before deletion and require a host-specific topology/notification migration
instead of claiming the recipe preserves them. Outgoing watches must be restored before watched work resumes.
If a deployment cannot quiesce, disclose the notification gap; retrying failed business messages is NOT a lossless
backfill of letters and may repeat external effects. Outcomes may expire or be hidden. No automatic replay is added.

Supply a tested Rust export-state conversion example using this release's serde_json: raw legacy payloads can all be
converted to payloadJson before restore, without discarding letters. It accepts only readable exports, validates the
result against the new state limits/schema before deletion, and never round-trips through a tool that rounds numbers.
A legacy payload deeper than 64 can be retained as text but still cannot be redriven; Discard remains possible. No
live repair is added: a raw legacy state deeper than 64 still cannot be written unchanged. Unreadable old exports need
an operator-specific recovery from the backup. No promise for arbitrary historical depth or already-lost float bits.

> **Correction (added by `dead-letters-upgrade-wording`).** The built example does not validate against the JSON
> Schema, and neither does `create_actor`, which `restore` calls. Before anything is deleted, `check` (run by
> `convert` and again by `reset`) refuses NUL in strings or keys, a state deeper than 64 levels, a state that does not
> decode as the 0.17.0 state, and an address repeated in the export. The test
> `the_schema_and_the_decoder_agree_on_the_payload_fields_of_a_letter` covers the payload fields of one letter only.
> It does not prove schema/decoder agreement on the whole state. This also qualifies "schema/serde agreement" under
> Proofs and scope. The original wording is kept as written.

## Proofs and scope

Conformance tests write/claim Failed with original depth64 on every backend; local and HTTP workers decode it on
runtime threads without storing the deep value in watcher state. Dead-letter tests pin raw61/text62/text64, raw null,
legacy/new marker shapes, exact numeric conversion, malformed/deep/NUL text and unchanged redrive metadata. Test an
encoded failed batch publication: keeping it works, but redrive can fail again for the original state-depth reason.
Depth-walk tests include empty arrays/objects and mixed nesting: empty containers still count. Test schema/serde
agreement and old-state conversion, hash change and maintenance backup/restore with watches.

Correct JournalEntry::Sleep's stale comment and regenerate description-only flow snapshots (hashes unchanged). Fix the
event lifecycle test to run Unsubscribe and assert surviving runs before deletion. Update architecture, layer guides,
operations and changelog; link relevant new conformance/runtime proofs in the existing G/S table where applicable.
Five implementation tasks plus finishing suffice. Tests use virtual time/TestActors; SQL legacy fixtures retain their
existing harness. Reconcile against the landed predecessor before building; do not archive specs during design.
