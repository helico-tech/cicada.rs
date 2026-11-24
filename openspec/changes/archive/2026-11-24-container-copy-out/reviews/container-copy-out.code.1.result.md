Verdict: pass

## Blocking

None. Tasks are ticked and in the diff. Version 0.20.9 everywhere, CHANGELOG entry present, no protected docs touched, handoff true to the code. Memory: skipped data is passed over by `tar`, kept files fill exact-size buffers within `max_bytes`, entries are capped by `max_files`. Half results: stream errors win over a `tar` success through the slot (`archive.rs` `read_archive`); a cut mid-entry is `UnexpectedEof`, so `Other`.

## Notes

1. Clean EOF at an entry boundary: `tar` 0.4.46 `next_entry_raw` returns `Ok(None)` when EOF comes before a header, without the end-of-archive blocks. A body that ends cleanly there gives a partial `CopyOut` as success. Unlikely: Go's writer closes the archive; hyper errors on a cut body. A reader wrapper that requires the zero blocks would close the gap.
2. The rustdoc and handoff say memory is "at most `max_bytes` plus `max_files + 1` paths, whatever the folder holds". This leaves out PAX and GNU long-name headers, which `tar` reads whole (`read_all`) whatever their size; only Docker's writer bounds them. The design said "plus one entry's headers", and the rustdoc should say the same.
3. Scenario "The container is removed during the call": only `settle` → `None` is tested. The second inspect → `Conflict` (`docker/mod.rs` `copy_out`) has no test. As the design planned; staging it is racy.
4. `a_big_skipped_entry_streams_past…`: the comment says "at most a few exist at once", but the test checks only that 64 chunks were made. A count of live chunks (a drop guard) would prove the bound.
5. `EntryKind`'s own `cfg_attr(allow(dead_code))` repeats the one on `mod copy` in `lib.rs`.

## Not checked

I ran no tests, E2E or gate, and did not check the gate evidence. I did not check Docker's answer for a path below a regular file (the fake gives an empty result).
