Verdict: not yet

## Blocking

1. **Breaking, called a patch.** Proposal Impact says "a patch". D6 adds `existing` to `WatchChange` (public, in `TurnEffects`) and `WatchChangeView` (public, `cicada::api`). Neither is `#[non_exhaustive]`, so struct literals break, as with `pinned_to` in 0.20.0. That also weakens D2's "avoid a breaking 0.22.0". Fix: state 0.22.0 with a "Changed (breaking)" entry in the proposal, D6 and the ADR; argue D2 on contract hashes alone.
2. **The PostgreSQL atomicity does not hold as written.** D4 and the ADR rely on "one transaction". At `READ COMMITTED`, a delete locks only watchers it already read. It can commit between the check and the watch insert, and no `Stopped` comes. Fix: put the target in the commit's address-ordered `FOR UPDATE` set. "Present" means that lock found its row, before `create_and_lock` inserts rows (the PG watch statements run after the creates). Add a PG test with a delete racing the commit.
3. **Undefined combinations.** Define and test: `existing: true` with a type target or with `set: false` in a remote body (`From<TurnEffectsView>` cannot fail today; say where it is refused and add a refused fixture); `watch_existing` then `unwatch`/`watch` of the same target; two calls in one turn; whether an older stored watch stays when the target is absent.

## Notes

- Docs handoff: `docs/architecture.md` (Watch and Notification terms), the G2 wording (a third event kind) and the `Turn::watch` rustdoc. The design names only the guarantees and the guide.
- Name the upgrade order (servers before workers): an old server refuses `existing`.
- The `Stopped` has the absent actor as `from`; say so in the rustdoc.

## Questions for the owner

- Q1: accept 0.22.0 for this batch? (Pick: yes.)

## Not checked

The commits of SQLite and the in-memory store; the `cicada-client` conversion; the rows of `docs/guarantees.md`. No tests run.
