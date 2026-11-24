# Docs handoff for container-inputs

Version: 0.20.9. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A container job can give its container folders and files before it starts. Additive: the actor's state and wire
format do not change (inputs are never stored), and no README or guide block, link, guarantee row or skill route
names the container actor's create step.

- `ContainerJob::inputs(&self, name: &str, request: &Self::Request) -> Result<Inputs, String>`, next to `spec`, with
  a default of no inputs: a job compiles unchanged.
- `Inputs { dirs: Vec<CopyDir>, files: Vec<CopyFile> }`, `non_exhaustive`, `Default`, exported from the crate root.
  `Debug` shows no file contents (`CopyFile`'s `Debug` shows only path, mode and length).
- The create step: build the spec and the inputs, check the inputs, `create`; with inputs, `inspect`, and only while
  the container is `Created`, `make_dirs` (with folders) and `copy_in` (with files); then `start`.

Proved by the unit tests in `crates/cicada-containers/src/actor/mod.rs` (`a_file_at_or_above_a_folder_is_refused`,
`folders_and_files_side_by_side_pass`, `a_path_the_rule_refuses_is_refused_with_the_runtimes_reason`) and by
`crates/cicada-containers/tests/actor.rs`
(`inputs_are_made_and_written_between_create_and_start_and_never_stored`,
`a_job_without_inputs_only_creates_and_starts`, `a_replayed_step_after_the_start_puts_no_inputs_in`,
`inputs_that_cannot_be_built_end_failed_before_the_create`, `an_input_the_runtime_refuses_ends_failed_without_a_start`,
`a_conflict_while_the_container_is_still_created_ends_failed`,
`a_conflict_with_the_container_gone_backs_off_and_the_next_poll_starts_it`,
`a_runtime_without_make_dirs_ends_failed_at_once`,
`a_container_gone_after_its_create_backs_off_and_the_next_poll_starts_it`,
`an_unavailable_daemon_while_copying_backs_off_and_the_next_poll_starts_it`). The rustdoc of `ContainerJob::inputs`,
`Inputs` and `ContainerActor` says the same; `Inputs` has a doctest.

## 1. Giving a container its inputs

**Where.** The container guide, when the docs session writes it (no guide covers `cicada-containers` yet): the
section on the container actor and its job, next to `spec`.
**Text.** A job gives its container folders and files with `inputs`, next to `spec`. Like the spec, the inputs are
built at each create from the request and never stored, so a secret file never reaches the store; the method is sync,
so load what it needs before the actor runs. The actor creates the container, makes the folders, writes the files,
then starts it, so a file can go into a fresh folder. A folder gets its mode only when it is missing (see
`make_dirs`): 0o777 lets a program that does not run as root write there.

```rust,ignore
fn inputs(&self, _name: &str, test: &Test) -> Result<Inputs, String> {
    let mut inputs = Inputs::default();
    let mut out = CopyDir::new("/out");
    out.mode = 0o777;
    inputs.dirs.push(out);
    inputs.files.push(CopyFile::new("/in/config.json", self.config_for(&test.commit)?));
    Ok(inputs)
}
```

(Not compiled from the guide; the doctest of `Inputs` builds the same value, and
`inputs_are_made_and_written_between_create_and_start_and_never_stored` runs it through the actor. `Inputs` and
`CopyDir` are `non_exhaustive`: outside the crate, start from `Inputs::default()` and `CopyDir::new(..)` and set the
fields. Make it a doctest of `examples/` if it goes in.)

The inputs go in only while the container is `Created`. A step that runs again after its container started (its
commit was lost) skips them; one still `Created` gets them again, which is safe: nothing that exists is made again,
and the same files are written. A job without inputs keeps the calls `create` and `start`.

## 2. Inputs the actor refuses

**Where.** The same section.
**Text.** Checked before anything is made, each ends the container `failed` with the reason, as a bad spec does:

- an `Err` from `inputs`, with its text;
- a path that is relative, has a `..` part or names no file (`/`), with the runtime's reason;
- a file at a folder's path or above it (dirs `/a/b`, file `/a`): writing it would replace the folder and what is in
  it. Paths are compared without empty and `.` parts, so `/out/.` and `/out` are one.

From the runtime, once the container is created:

- `InvalidSpec` from `make_dirs` or `copy_in` (a mount target, a tmpfs, a read-only mount, a bad mode) ends it
  `failed`: a retry gives the same answer;
- `Conflict` (a file where a folder is asked) ends it `failed` when a new inspect still finds the container `Created`;
  otherwise it counts as a runtime error;
- `Other` from `make_dirs` ends it `failed` at once: a runtime written before `make_dirs` cannot make folders, and a
  retry cannot help;
- any other error, and a container missing right after its create, counts and backs off; the next poll creates it
  again if needed, puts the inputs in and starts it.

A container that ended `failed` after its create stays until `Remove`, like one whose start was refused.

**Caveats.**
- As with `spec`, an `Err` from `inputs` on a step that runs again after its container started ends it `failed` and
  leaves it running until `Remove`.
- Inputs cannot change after start: files go in only before it.

## 3. Secret files

**Where.** The same section, or the secrets paragraph of the container guide.
**Text.** Put secret files on the container's own file system, not on a volume: `Remove` deletes the container with
its files, while a named volume outlives it. A tmpfs path is refused, as the tmpfs is mounted only at start and would
hide the file.

## Blocking

None.

## Status

Ran `cargo test -p cicada-containers --all-features` (131 unit, 32 actor, 7 doc tests passed) and
`cargo test -p cicada-examples --test docs`; the finishing checks and the gate are in the change's last task. All
prose above is proposed text; none is written into the guides yet.
