Verdict: pass

## Blocking

None.

## Round 1, finding 1: fixed

Fix (a) was taken. `git diff 0ffdbfe..4ddb330` touches only prose, the `MAX_LINKS` doc comment and one test name (`eight_links_in_the_walk_are_followed_and_a_ninth_is_a_conflict`). No code changed behaviour. The `make_dirs` rustdoc in `runtime.rs`, the `DockerRuntime` doc in `docker/mod.rs`, `CHANGELOG.md`, `docs-handoff.md` §2–3 and the new requirement "Links on Docker are followed to a folder" all say the same thing now:
- the walk follows at most 8 links in all;
- Docker resolves a chain itself, so a chain counts as one link;
- a link loop fails with `Other`.

That matches the code. `stat` in `docker/mod.rs:219` maps only a 404 to `Missing`. Every other error goes through `fail`, then `kind_of`, so a 500 becomes `Other` (`map.rs:30`). Round 1's note 1 ("in a row") is fixed too, and the old wording is gone from the crate and the change. The scenario that moved, "A folder through a linked parent", keeps its E2E test. The version is still 0.20.9, and no protected docs are touched.

## Notes

1. The new requirement says a loop "SHALL fail the call with `Other`", but no scenario or test pins this. That doesn't block, because only scenarios need tests. A Docker E2E step with `loopa <-> loopb` would pin it if the owner wants that.
2. The design's line "split out by code review 1" is history, not design. It's harmless.

## Not checked

I didn't run the suites, the E2E or the gate. I relied on the builder's evidence for 4ddb330, and its E2E evidence from 0ffdbfe, since only comments changed after that.
