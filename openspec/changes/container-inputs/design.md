# Design

The container actor goes from `create` straight to `start`, so a host can give a program its inputs only as
environment variables. This change lets a job give folders and files, which the actor puts in between create and
start. It uses `make_dirs` and `CopyDir` from `container-make-dirs` (split out by this change's design review) and
`copy_in`.

```rust
// ContainerJob, next to spec():
fn inputs(&self, name: &str, request: &Self::Request) -> Result<Inputs, String> { Ok(Inputs::default()) }
pub struct Inputs { pub dirs: Vec<CopyDir>, pub files: Vec<CopyFile> }   // non_exhaustive, Default
```

## Decisions

1. **The job returns one `Inputs`, by a method called `inputs`.** Folders and files are two lists of one struct; the
   actor makes the folders first, then writes the files, so a file can go into a fresh folder. The name is `inputs`,
   not `files` as first agreed, as it returns both. Like `spec`, it is sync and built at each create, so secrets never
   reach the store; the default gives no inputs, so hosts compile unchanged. Not taken: `Vec<Input>` with an enum
   entry (one more type and a match for every host, and the order of mixed entries would matter); two job methods
   (two places to keep in step).
2. **The create step copies only into a `Created` container.** With inputs, the step runs: build the spec and the
   inputs, `create`, `inspect`, then `make_dirs` (when there are folders) and `copy_in` (when there are files) only
   when the status is `Created`, then `start`. A replayed step whose earlier run already started the container finds
   it running or ended and skips the copy; `start` is then a no-op, as now. A container that is still `Created` gets
   the inputs again, which is safe: `make_dirs` makes nothing that exists, and `copy_in` writes the same files.
   `Missing` right after `create` (removed by someone else) counts as a runtime error and backs off; the next poll
   runs the step again (design review note). Without inputs the step makes no `inspect`, `make_dirs` or `copy_in`
   call, so its calls are unchanged. Not taken: calling `copy_in` and reading its `Conflict` as "already started"
   (`Conflict` also means a missing container); a new phase between create and start (a state change for a step
   that repeats safely anyway).
3. **Inputs checked before anything is made.** An `Err` from `inputs` ends the container `failed` with its text
   before `create`, as a bad spec does. So does a file at or above a folder: at a folder's own path or at one of its
   parents (dirs `/a/b`, file `/a`), as Docker's `copy_in` would replace the folder just made, and what is in it.
   Paths are compared after the shared path rule's parsing, so `/out/.` and `/out` are one; a path that parsing
   refuses ends it `failed` here too, with the reason the runtime would give (design review 2, blocking 1). Not
   taken: leaving the clash to the runtime (Docker replaces the folder silently).
4. **Failures from the runtime.** `InvalidSpec` from `make_dirs` or `copy_in` ends the container `failed` with the
   reason: the job's mistake, and a retry gives the same answer. `Conflict` from either has two meanings, a file
   where a folder was asked or a container that is no longer `Created`, and the first `inspect` can be out of date;
   so the actor inspects again: still `Created` means the job's mistake and ends it `failed`; anything else counts
   as a runtime error and backs off (design review 2, blocking 2). `Other` from a runtime without `make_dirs` (the
   trait's default) ends it `failed` at once when the job has folders: the call is unsupported, so a retry cannot
   help, and backing off `max_errors` times would take minutes. Any other error counts in the state and backs off.
   The created container stays until `Remove`, like one whose start was refused. `container-limits-and-kinds` gives
   these ends the kind `notStarted`. As with `spec`, an `Err` from `inputs` on a replayed step whose container
   already runs ends it `failed` and leaves it running until `Remove`; the rustdoc of `inputs` says so.
5. **Secrets stay on the container's own file system.** The shared path rule refuses a tmpfs path, so a secret file
   cannot be lost before start. The rustdoc of `inputs` says to put secret files on the container's own file system,
   not on a volume: `remove` deletes the container's layer, but a named volume outlives it.

## Spec deltas

`container-actor`: *Inputs are copied in before start* and *Refused inputs end the container* are added. The actor's
state and wire format do not change (inputs are never stored), so no contract snapshot or ADR changes.

## Tests

Actor tests on virtual time with `TestActors` and the fake (`#[tokio::test(start_paused = true)]`): the calls in
order with inputs (create, inspect, make_dirs, copy_in, start) and unchanged without; an `Err` from `inputs`, a file
at a folder's path, a file at a folder's parent and `/out/.` with `/out` end `failed` with no create; a replayed step
on a running container skips the copy; `InvalidSpec` from `copy_in` ends `failed`; `Conflict` from `make_dirs` with
the container still `Created` ends `failed`, and with it gone backs off; `Other` from `make_dirs` ends `failed` at
once; `Missing` after `create` and `DaemonUnavailable` from `make_dirs` back off, and the next poll copies and
starts; the stored state holds no input contents. No E2E: the actor runs on the fake, and Docker's `make_dirs` and
`copy_in` have their own E2E cases.

## Docs it breaks

None: no README or guide block, link, guarantee row or skill route names the container actor's create step.
`docs-handoff.md` carries `inputs`, `Inputs`, the order (folders, then files, then start) and the secret-file advice
for the container guide.

## Version

0.20.9, with the rest of the container line; released when the line, through `container-limits-and-kinds`, has
landed.
