# Design

## Register without hosting

Add `ActorSystem::register_contract(&Contract) -> Result<RegisterContractResult, OpError>`. The result is
`Registered(RegisteredType)`, `Conflict { existing: RegisteredType, requested_hash: String }`, or
`InvalidContract(String)`, exported with the runtime's other operation results. It works on an unstarted system,
including one with no hosted types. It neither initializes a node nor adds a handler, a node row or a change hint.
Do not expose Backend or its RegisterResult: the public operation owns validation and returns the public read model.

Rebuild the incoming Contract with `Contract::new` from its name and schemas before storing it. Even a Contract made
with `Contract::of(TypeDef)` can carry a supplied hash; the new operation always computes its own. Use the existing
atomic Backend::register, then the resolver's conversion of the returned TypeDef, without a lookup-before-write.
Same hash returns the stored contract, including its original descriptions. Different hash returns Conflict, with
no replacement, even if no actor exists. Two conflicting concurrent registrations admit exactly one contract.
Fix the resolver cache as part of this path: each hash entry holds the source JSON alongside its parsed Contract.
Reuse it only when that JSON equals the returned TypeDef.contract; otherwise parse and replace the entry. Contract
equality alone compares hashes and is insufficient. This also keeps reads fresh after another system deletes and
re-registers a type with new descriptions; invalidating only this system's cache on removal would not suffice.

Local initialization remains on its existing path and retains StartProblem::ContractConflict. A local node started
after remote registration accepts the same generated hash and refuses a different one. Registering a contract does
not make its type appear in hosted_types or cause a send-only server to claim its messages.

## Request and response

Add `api::RegisterTypeRequest { state: Value, messages: BTreeMap<String, Value> }` and PUT /types/{name} on the full
router, operationId `registerType`. Both fields are required. The decoded path supplies TypeName; there is no name,
type or hash field in the body. The envelope uses deny_unknown_fields, while schema documents keep their keywords.
The read-only router answers its ordinary 405 for PUT on this existing read path.

Example: `PUT /types/counter` with
`{"state":{"type":"object"},"messages":{"Add":{"type":"object"}}}` answers the existing TypeView:
`{"name":"counter","hash":"sha256:…","state":{"type":"object"},"messages":{"Add":{"type":"object"}}}`.
Use 200 for both a new and an equal registration: the store intentionally does not distinguish them. A lost response
can be retried with the same body. Return what Backend::register accepted, not a second read that could race deletion.

Reuse request::body and the configured body-byte limit. No content-type rule beyond the existing parser is added.
Empty/malformed/non-object bodies, missing fields, non-object messages, unknown envelope fields and an invalid path
name are 400 bad-request. Duplicate JSON members follow the existing Value parser (last value wins); do not add a
different parser for this route. In particular a supplied hash is refused rather than ignored.

## Contract validation

The public operation checks before touching the store: the state and each message schema root are an object or a
boolean; each message name obeys the current send-name limit of 200 characters and contains no NUL; and no schema
string or object key contains NUL (PostgreSQL cannot store it). Empty message maps and boolean schemas are valid.
Message names are not actor TypeNames: use the existing send-name rules, including their allowance of an empty name.
These failures are InvalidContract and HTTP 422 invalid-contract, including explicit null schemas.

Walk each complete schema value for NUL and a schema-specific maximum of 120 nested arrays/objects, counting the
root container as one and a scalar as zero. At 121 return InvalidContract before the store. The stored contract and
request add at most two levels (122); TypeListView adds at most four (124), below the SQL/client JSON parser limit
of 128. Do not use the 64-level actor-value limit. Check before recomputing the canonical hash. Contract::new,
from_json, of and local hosting keep their existing semantics; the new bound belongs to this registration entry.
HTTP retains its parser/body-byte limits. A generated contract above 120 needs simplification to register remotely;
this change does not repair existing over-deep registrations made through the older local hosting path.

Do not compile or fetch schemas at registration. Nested schema semantics, references and keyword support keep the
ADR 0026 policy: the HTTP door enforces a usable schema, warns and skips an unusable one, and loads nothing external.
A valid registration is not proof that a handler exists or that its schema accepts any possible value. A worker
must later register its exact generated contract, including titles and definitions, rather than a hand approximation.

## Errors and client

Conflict maps to 409 contract-conflict with the ordinary Problem envelope; detail names the type and the registered
and requested hashes. Do not add structured fields to all Problems or change the existing startup error. Store
failures keep the usual 503 unavailable / 500 corrupt mapping. A valid request after Store::close gets 503.

Add `ActorClient::register_type(&Contract) -> ClientResult<TypeView>`, sending only state and messages to the path
from the contract's TypeName. The client returns the server's stored view and uses ClientError::Problem for 409/422;
it does not parse detail text or silently replace a contract. Future worker startup translates that problem in its
own design. The client sends no hash, so the server always recomputes it even for a Contract loaded from a TypeDef.

## Compatibility and scope

ADR 0036 records the new wire operation. No schema migration, new dependency, guarantee change or Backend change.
Registration uses the full router's existing host authentication. It is a write capability, not a worker identity or
authorization boundary. Existing removal rules still apply: delete only without actors, then register anew.
No state/schema upgrade, hot reload, new handler, automatic actor creation or lease endpoint is included.

## Verification and documentation

Core tests cover an empty/unstarted system, computed rather than supplied hashes, validation without writes, original
descriptions, peer delete/re-register/read, conflicts, concurrency and subsequent local initialization. Keep existing
backend registry conformance tests; no backend implementation changes are needed. Test a generated Rust contract and
120-level state and message schemas through registration, SQL reopen/read, equal retry and HTTP/client decoding;
121 levels fails without writes. Run the boundary on SQLite and configured PostgreSQL, not just memory.
HTTP/client tests cover all response classes,
full/read routers, unknown envelope keys, body limit, close, and create/send using the newly registered schemas.
The latter leaves a message pending until an actual matching host starts; a mismatching payload is still refused.

Update the core and HTTP guides with tested examples, the architecture's registration/HTTP description, axum's route
table, generated OpenAPI and changelog/version. Four implementation tasks suffice: core, HTTP, client, docs; then
the standard finishing checks. No runtime tests are run during design review.

## Review resolutions

The design review found unreadable over-deep schemas and description-stale resolver entries. The 120-level bound
with SQL/read/response boundary tests and full-source-JSON cache comparison resolve both; no owner decision remains.
