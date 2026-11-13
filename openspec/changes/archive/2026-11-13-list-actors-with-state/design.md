# Design

## Scope

One listing that returns actor state without N extra reads. The owner's Opus 5.5 adviser settled the choices below.
Four implementation tasks plus finishing checks. ADR 0033 records the additive HTTP contract; no schema migration.
The existing address listing, CLI and terminal UI stay unchanged. The following change-feed proposal can use this
listing for resync, but adds no requirement here for a durable version history or a snapshot across pages.

## Decisions

### A typed sibling, reusing the single-actor snapshot

Add ActorSystem::list_actors_with_state(&self, prefix: &str, cursor: Option<&PageCursor>, limit: usize)
-> Result<Page<ActorSnapshot>, OpError>. Keep list_actors's signature and address-only reads. Each row has the existing
ActorSnapshot fields: address, state, version, created_at, paused and parent. Share get_actor's ActorRecord conversion
through a private helper; do not export a new conversion or row type. Raw JSON is returned without schema/actor decoding.
Not chosen: a boolean on list_actors (breaks the typed return/signature), a new minimal row (duplicates ActorSnapshot),
or list_actor_snapshots as a name (suggests a snapshot spanning pages).

### One set-based read

Add private Backend::list_records(prefix, cursor, limit) -> StoreResult<Page<ActorRecord>>, through the store handle
and all three backends. Each SQL implementation performs one actor SELECT per successful read attempt in its normal
read transaction; memory takes one lock and clones the records. Fetch at most limit + 1 rows for page_of. No per-row
get, registry lookup or host callback. Name projection columns explicitly, as get does; never SELECT *. Reuse the get
row decoder and list's address ordering, prefix predicate and
cursor bound logic within each backend; do not introduce a generic query framework or make address-only list load state.
SQLite uses substr, PostgreSQL starts_with plus the current bounds, memory starts_with; no LIKE wildcard interpretation.
States and versions in a row come from the same committed record, and a page observes one read snapshot. An uncommitted
handler state is never visible. The transaction may retry by existing backend rules; this is not a count of BEGIN/COMMIT.
Not chosen: list then get each item (N+1 and delete races), a materialized view or a new index (already stored columns).

### Existing keyset rules

The prefix is a literal prefix of the canonical address, empty for all. `a` includes both `a/one` and `ab/one`; `a/`
selects only that type. Percent and underscore remain literal characters; HTTP query decoding happens once as today.
Address order and the opaque next cursor match the address listing. Reuse check_limit/page_of: core rejects zero,
with no new upper cap; HTTP accepts 1..1000 and defaults to 100, also when the client omits limit. Return next only if the lookahead row exists; an empty page has no next. A cursor before the
prefix begins at the prefix, and a cursor beyond all matches returns empty, as in list. Blank HTTP cursor means absent.
No new cursor format or validation, and no promise across pages: updates may appear at later versions, new addresses
before the cursor are missed, deleted actors disappear, and delete/recreate can reset versions. Versions count state
writes in one actor life, not changes of paused/parent or a global watermark. Clients keep the same prefix while paging.
Not chosen: smaller state-page limits (outcome pages already carry large values), or a byte cap (new truncation policy).
Document that large states call for a smaller limit; no new response-size guarantee is added.

### A fixed HTTP response type

Add GET /actors/states to both full and read-only routers, operationId listActorStates, Query<ListQuery>, response
PageView<ActorView> (ActorViewPage). Use ActorView::from on the snapshots, with the same fields/encoding as a single
GET: address, type, state, version, createdAt, paused and parent (null for independent actors). Empty result is
200 {"items":[],"next":null}. Existing GET /actors retains its response and OpenAPI operation. /actors/states has one
segment after actors; /actors/{type}/{key} has two, so an actor with type `states` stays addressable.
Reuse Params: first duplicate value wins, unknown keys are ignored, malformed query/invalid limit is 400, prefix
defaults empty. No state flag or boolean parser. Error bodies follow application/problem+json; store unavailable is
503, corrupt data is 500, and no partial page is returned. Both routers remain behind the host's authentication layers.
Not chosen: state=true (a response union on every generated listActors client), or a top-level /snapshots route.

Add ActorClient::list_actors_with_state(prefix: &str, cursor: Option<&str>, limit: Option<usize>) returning
ClientResult<PageView<ActorView>>, using the existing listing query helper and /actors/states. Do not fall back to N+1
when talking to an old server: its ordinary error is returned. Upgrade server before using the new method.

### Stored values and compatibility

Read existing data under the same rules as get_actor. ADR 0029 checks writes, not reads: older deep states are neither
truncated nor newly rejected. A malformed fetched row fails the page, including a malformed lookahead row under the
existing decode-before-page_of pattern. States written under the 64-level limit reach at most 67 levels in this view
(page object, items array, row object); the documented maximum of 69 across HTTP views still holds. Legacy deeper
values may exceed a client's parser limit and make the whole page unreadable, so document the existing caveat here too.
No actor contract/hash or stored schema changes. The route/method are additive and ordinary clients are unaffected.

### Verification

Conformance on memory, SQLite and PostgreSQL compares rows/cursors with list/get, literal prefixes and limit errors.
Virtual-time runtime tests cover state versions after commits, parent/paused metadata and between-page mutations.
HTTP/client tests cover row encoding, both routers, empty/last pages, limits, duplicate queries, route precedence and
unchanged address listings. Extend the existing legacy-depth fixture; regenerate OpenAPI. Guides include a compiling
example. Keep architecture wording specific to this listing; do not promise one-read behaviour for unrelated APIs.

Review 1: ready for tasks, no blockers. Clarified core/HTTP limit parity: the shared check_limit rejects only zero;
the new method adds no core maximum, while HTTP keeps its 1000 cap. Added the explicit SQL-column requirement.
