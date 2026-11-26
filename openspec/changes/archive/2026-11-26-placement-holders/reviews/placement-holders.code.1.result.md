Verdict: pass

## Blocking findings

None.

## Notes

1. `placement.rs` `follow`: the only source of a new target is an `Ask`, and that commit also sends the answer to the
   target, which fails the commit when the target is gone. So the "absent" branch of `watch_existing` cannot fire in
   practice; a plain `watch` would act the same here. This is harmless and safe for later code paths. But design
   decision 1 says `watch_existing` "closes every case", which overstates what it adds.
2. Re-created target: a `Stopped` of an old incarnation frees by address. If it were handled after a new
   incarnation's `Ask`, it would free the new holder too. Mailbox order prevents this, because the `Stopped` is written
   at the delete, before any later ask. A one-line comment on `gone` would make that reliance explicit.
3. `a_target_deleted_before_its_grant_gets_no_place`, second half: r3 is in fact granted from the line in the release
   turn and freed by the `Stopped` that follows. Its `Notify` then finds no holder. "Gets no place" holds in the sense
   that it is never told, and the place ends free. The test comment could say that.
4. The departure (only a store-written `Stopped` counts) is right: without it a client could free other holders'
   places. The changelog states it; the handoff does not list it as a caveat. That is optional.

## Checked

- All 4 tasks are ticked and their work is in the diff.
- Each of the 5 spec scenarios, and the self-ask refusal, has a test.
- Version 0.24.0 matches in `Cargo.toml`, both skills and `plugin.json`.
- `CHANGELOG.md` has its entry.
- The contract reference was regenerated (the `Ask` description).
- No protected docs are in the diff.
- The handoff matches the code, and its anchor text exists at `docs/guide/actors.md:612`.
- `gate.py status 6dbdfa9b` shows a full pass.

## Not checked

- I did not run any tests; I relied on the builder's run and the gate record.
- No docs or e2e record exists for this head.
- I did not check against a live PostgreSQL store how the store orders notifications when one commit writes several.
