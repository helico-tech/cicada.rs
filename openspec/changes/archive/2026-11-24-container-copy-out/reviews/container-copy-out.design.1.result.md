Verdict: not yet

## Blocking

1. **The file-count stop has two readings.** design.md decision 1 and the runtime spec (*within limits*): "At `max_files` entries the copy SHALL stop and add one skipped entry". With exactly two files and `max_files` 2, is a false "rest was not read" entry added? Say it is added only when one more countable entry follows, and add an at-the-limit scenario. Decide if zero limits are `InvalidSpec`.
2. **Error paths have no scenario.** docker spec: an error mid-stream (whole call fails with its kind, no partial result), and a container removed during the call (404, then `Conflict`).

## Notes

1. bollard's error has no `Into<io::Error>`, so `StreamReader` needs a `map_err`. Say how 404 and `DaemonUnavailable` get back out through tar: look at the first chunk before tar, or wrap and downcast.
2. Memory is bounded: bollard streams hyper frames, and tar 0.4.46 skips in 32 KiB reads. But tar reads PAX and long-name data with an unbounded `read_to_end`; Docker's PATH_MAX and xattr limits cap it, so add "plus one entry's headers" to the bound. Read kept files with `with_capacity(size)` and `read_exact`, not `read_to_end`.
3. When the caller drops the future, the `spawn_blocking` reader goes on reading the archive to its end. Stop it when the result channel closes.
4. After the end, Docker returns image files under a tmpfs target. Write "files written to the tmpfs are gone", not "empty".
5. `container-inputs` may add a folder kind to `CopyFile`, which `copy_out` returns; prefer a separate `CopyDir` there.

## Questions

None.

## Not checked

What Docker does when a file grows or vanishes mid-walk (moby logs some errors and goes on, so a result could come back short without an error). Where hard-link entries fall. Nothing was built or run.
