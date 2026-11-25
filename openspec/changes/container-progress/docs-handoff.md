# Docs handoff for container-progress

Version: 0.23.0 (BREAKING; a new unreleased section above 0.22.1). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

A container job's program reports its progress through a file, and the container actor sends it to the owner (ADR
0065). New and changed public items of `cicada-containers`:

- `ContainerMsg::Start { to, message_type, progress_type: Option<String> }` (wire `progressType`, optional) and
  `EndTarget { to, message_type, progress_type }`. `progressType` is bounded like `messageType` (1 to 200 characters,
  no NUL); one out of bounds is refused for good with nothing saved. The first `Start` wins, as for the end.
- `ContainerMsg::start_with_progress(&target, "Ended", "Progress")`, beside `start`.
- `ContainerProgress { key, seq, text }`, the payload of a progress message; a host's family takes it as a variant such
  as `Progress(ContainerProgress)`.
- `ContainerJob::PROGRESS_FILE`, an associated constant, default `/tmp/cicada-progress`.
- `ContainerState` gains `progress_seq: u64` and `progress_hash: Option<String>` (wire `progressSeq`, `progressHash`).
- `sha2` is no longer optional in the crate.

Proven by the progress tests in `crates/cicada-containers/tests/actor.rs`
(`three_changes_over_polls_are_sent_with_seq_one_two_three`, `the_last_change_before_the_exit_is_sent_before_the_end`,
`the_ends_of_a_container_that_ran_read_the_file_once_more`, `without_a_progress_type_no_file_is_read_and_no_env_is_set`,
`the_progress_env_is_set_after_the_jobs_spec`, `a_file_that_cannot_be_read_sends_nothing_and_is_no_error`,
`a_nul_and_invalid_utf8_become_the_replacement_character`, `an_empty_text_is_sent_only_after_one_that_was_not`,
`a_refused_progress_send_moves_seq_and_does_not_block_the_end`, `a_start_with_a_progress_type_out_of_bounds_is_refused`,
`a_progress_path_the_rule_refuses_ends_not_started_and_makes_nothing`, `ends_without_a_run_read_nothing`,
`the_progress_payload_passes_its_sample`), the test job's contract snapshot `tests/contracts/test.container.json`, and
the doc example of `ContainerMsg::start_with_progress`.

## 1. Report progress

**Where.** `docs/guide/containers.md`, under `## The container actor`, a new section such as `### Progress`, after
`### The end`.
**Text.** A job's program can report its progress while it runs. Ask for it in the `Start`:

```rust
let start = ContainerMsg::start_with_progress(&owner, "TestEnded", "TestProgress")?;
```

(compiled in the doc example of `ContainerMsg::start_with_progress` in `crates/cicada-containers/src/actor/wire.rs`;
the owner's family takes `TestEnded(ContainerEnded)` and `TestProgress(ContainerProgress)`.)

- **The file.** The actor sets `CICADA_PROGRESS_FILE` in the container's environment to the job's `PROGRESS_FILE`
  (default `/tmp/cicada-progress`; override the associated constant in the `impl ContainerJob`). It is set after the
  job's `spec`, which cannot change or hide it. The program writes its progress there, in any text format: the actor
  does not parse it. A `PROGRESS_FILE` that is relative, has a `..` part or names no file ends the container `failed`
  with the kind `notStarted` before anything is made.
- **Write it atomically.** Write to a temporary name in the same folder, then rename onto the path (`mv`). A file read
  while it is written can come back half written; it is sent as it is, and the next poll corrects it.
- **Not on a tmpfs.** The actor reads the file with `copy_out`, which cannot see a tmpfs. With a read-only root file
  system and a tmpfs on `/tmp`, set `PROGRESS_FILE` to a path on a volume.
- **When it is read.** At each poll of the running container (every `poll_interval`) and once in the turn that records
  an end for which the container ran: `exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped` and
  `timedOut`. Not for `missing`, `notStarted` or `runtimeErrors`.
- **What is sent.** When the text changed since the last progress sent, the owner gets
  `{"key": "t1", "seq": 3, "text": "{\"done\": 40, \"total\": 120}"}` as `progressType`, `seq` counting from 1. At most
  64 KiB is read; the text is UTF-8 with each invalid sequence and NUL replaced by U+FFFD. An empty file is sent only
  after a progress that was not empty.
- **Best effort.** A missing file, one over 64 KiB or not a regular file, and a failed read send nothing and are not
  runtime errors: progress never ends a container. A progress the target's type refuses (not registered, or no such
  variant) is not retried and does not hold up the end.
- **Order.** The last progress is sent in the end's commit, before the end. A progress the owner fails to handle is
  retried after a backoff, so it can arrive after a later progress or after the end. **Drop a progress whose `seq` is
  not above the last one you handled, and any that arrives after the end.**

A short shell example for the program, if wanted:

```sh
printf '{"done": %d, "total": %d}' "$done" "$total" > "$CICADA_PROGRESS_FILE.tmp" && mv "$CICADA_PROGRESS_FILE.tmp" "$CICADA_PROGRESS_FILE"
```

**Caveats.** Each running container with progress costs one `copy_out` per poll. Progress lives only in the messages
and in the state's `progressSeq`/`progressHash`; the text itself is not kept in the actor's state.

## 2. Host and start: the third message type

**Where.** `docs/guide/containers.md`, `### Host and start`, step 3.
**Text.** Add: or `ContainerMsg::start_with_progress(&target, "Ended", "Progress")?` to get progress too (see
`### Progress`); the target's family then also takes `Progress(ContainerProgress)`.

## 3. Upgrade to 0.23.0

**Where.** `docs/how-to/upgrading.md`, a new `## Upgrade to 0.23.0` above `## Upgrade to 0.22.0`.
**Text.** **Version 0.23.0 changes container contracts.** `Start` and `EndTarget` gain the optional `progressType`, and
the state gains `progressSeq` and `progressHash`, so the contract hash of every container job type changes. A node
requires the exact registered hash, so for each such type follow `Changing a contract` (operations guide): a versioned
type name for new runs while old actors finish on old hosts, or a maintenance window (stop every host of the type,
delete its actors and then its registration, start the new hosts). Send `Remove` to a container actor before deleting
it, or its container stays. A target type changes only when it adds a variant that takes a `ContainerProgress`.

In Rust, add `progress_type: None` to every `ContainerMsg::Start` and `EndTarget` struct literal, such as in your
tests, and bind or ignore it (`..`) in exhaustive patterns; add `progress_seq: 0, progress_hash: None` where you build a
`ContainerState` by its fields (`ContainerState::new` needs no change). The store schema, the HTTP API, the ready-made
actors' contracts and the guarantees do not change.

**Caveats.** `docs/reference/contracts` has no container contract (the job type is the host's), so nothing there
changes.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --doc` and `cargo test -p cicada-examples --test docs` on this branch: both pass, so
no guide block builds `Start` or `EndTarget` and nothing blocks. All of the above is proposed text; no prose is written
yet.
