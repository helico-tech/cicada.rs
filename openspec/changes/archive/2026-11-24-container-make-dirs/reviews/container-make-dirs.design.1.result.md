Verdict: not yet

The split answers earlier findings 2 and 3: existing folders are skipped, a file at the path is `Conflict`.

## Blocking

1. **A linked parent is refused (design decision 4, runtime delta).** bollard 0.21 `get_container_archive_info` sends a HEAD and decodes `X-Docker-Container-Path-Stat`. Docker fills it from an lstat: a link gives a link mode and a `linkTarget`, never `ModeDir`. On 404 the upward walk stats each parent. In Debian-based images `/var/run` is a link to `/run`,. So `make_dirs("/var/run/app")` would be `Conflict` on Docker but `Ok` on the fake. Decide: follow a parent's `linkTarget`, which is an absolute path in the container, and accept it when it is a folder. Add an E2E scenario.
2. **A path below a file (decision 4, Limits).** Docker probably answers ENOTDIR with a 500, not a 404, so the walk never starts. Then the "file at a parent is `Conflict`" scenario fails on Docker. Decide what a non-404 stat error does. For example, walk upward anyway: `Conflict` if a parent is a file, otherwise the original error.
3. **Overlapping names (decision 2).** `[/a/b, /a 0o777]` names `/a` and also makes it as a parent of `/a/b`. Is `/a` made 0o777 or 0o755? Nothing says. The same path twice with two modes is also undecided. Decide that a named folder keeps its own mode in any order. Make different modes for one path `InvalidSpec`, or say which one wins.

## Notes

- The requirement title in design "Spec deltas" ("only before start") differs from the delta's title.

## Questions

- Does Factory ever name a folder under `/var/run` or `/run`?

## Not checked

Moby's real stat and extract behaviour. Docker's side is from memory. Mount points on a created container. The whole fake.
