Verdict: not yet

## Blocking

1. **Proposal contradicts design.** `proposal.md` What Changes still says `files(...) -> Vec<CopyFile>`; design decision 1 says `inputs(...) -> Inputs`. Rewrite that bullet.
2. **An existing folder: owner, not only mode.** Decision 2 and the runtime delta say an existing folder "gets the mode". A Docker folder entry also chowns it to uid/gid 0 (moby's extractor, from memory). So `make_dirs("/home/app")` on an image folder owned by user 1000 makes it root's, which the program then cannot write. The fake hides this. Say it in the requirement, rustdoc and an E2E scenario, or skip existing folders.
3. **File/folder clash, no scenario.** `make_dirs` over a file, or `copy_in` of a file over a made folder. Docker's upload, as called (no `noOverwriteDirNonDir`), silently replaces it, a folder with its contents. The fake has no rule. Name the result (replace, or refuse with a kind), make both runtimes agree, add a scenario.
4. **Size.** Two behaviours across three specs: a runtime method (trait, fake, Docker, E2E) and the actor's create step. Split: `container-make-dirs` first, then `container-inputs` for the actor only.

## Notes

- The tar must hold only the named folders, never parents, or `/tmp/out` resets `/tmp` from 1777 to 0755. Add a `map` test.
- The mount-target refusal means `make_dirs` cannot set a volume root's mode. Say so in the rustdoc.
- Say whether a mode above 0o7777 is `InvalidSpec`.
- Decision 3: `inspect` giving `Missing` after `create` falls to `start` and a backoff; say so.
- `container-limits-and-kinds`: refused inputs should get kind `notStarted`; add a line there.

## Questions

- Does Factory need a 0777 volume root, or a folder owned by a non-root user?

## Not checked

Docker's real chown, implied-parent and overwrite behaviour (from memory, not run); `container-logs` in its worktree; the whole fake.
