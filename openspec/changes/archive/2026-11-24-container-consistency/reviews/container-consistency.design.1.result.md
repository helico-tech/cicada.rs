Verdict: ready for tasks

## Blocking findings

None. The code references hold: the fake's `under` accepts an exact target, `map::uploads` needs a mount strictly above the path, the fake accepts `/`, `sweep` treats 404/409 differently per kind, and `list` fails through `status`. No actor calls `list` or `sweep`.

## Notes

1. Size: three behaviours in one change (AGENTS.md asks for one). It still fits in 5 tasks. A split, if wanted: the `copy_in` path rule (shared module and E2E) alone, then `sweep` with `list`.
2. A gap of the same kind, not named: a tmpfs on a writable root. The fake writes `/tmp/a`. Docker uploads it before start, and the tmpfs then hides it, so the file is silently lost. Refuse paths at or below a tmpfs target in the shared rule, or add a line to `container-inputs`.
3. The `container-runtime` sweep delta says "already gone", but the design also leaves out a container that answers 409 (another caller is still removing it). Say "gone or being removed", or add 409 to the scenario.
4. `cicada-containers` shipped in 0.20.7. Hosts will see the changes to `Swept.containers` and the refused exact target on the fake. Name them under Changed/Fixed in CHANGELOG.
5. The fake stores `file.path` as given. Once paths are normalised, `/work/./a` and `/work/a` are two files on the fake but one on Docker. Store the normalised path.

## Questions for the owner

- Version: does "released with the rest of the container line" mean waiting for `container-limits-and-kinds`, or the next batch after 0.20.8?
- Split in two (note 1)?

## Not checked

What a 409 on Docker's forced remove means, and whether Docker refuses a file over a mount point. No builds or tests run. The E2E file was not read in depth.
