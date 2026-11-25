# Design

The third change of the container-outputs line. A job's program writes its progress to a file in the container; the
container actor reads it while the container runs and once more at its end, and sends it to the owner as numbered
messages, only when it changed. Factory does this in its runner today.

```rust
// The owner asks for progress next to the end:
Container::<Tests>::named("t1").send(ContainerMsg::start_with_progress(owner, "TestEnded", "TestProgress"))?;
// The program in the container: write to a temp name, then rename onto $CICADA_PROGRESS_FILE.
// The owner gets {"key": "t1", "seq": 3, "text": "{\"done\": 40, \"total\": 120}"} as a TestProgress message.
```

## Decisions

1. **In the actor's polls; a contract change.** The actor polls a running container anyway, and a progress message
   needs a turn to be sent. Reading a small file with `copy_out` in a poll turn is cheap. The actor's state gains
   `progress_seq: u64` and `progress_hash: Option<String>` (SHA-256 of the last sent text, hex; `sha2` stops being
   optional in `cicada-containers`), and `Start` and `EndTarget` gain an optional `progressType`, so the contract of
   every container job type changes (ADR 0062's precedent): **BREAKING**, one minor version (0.23.0). Rust code that
   builds or matches `Start`, `EndTarget` or `ContainerState` changes too. `container-output-collect` is likely to
   change the end report as well; when it does, both ship in that one release, else this ships alone (the ADR does
   not tie them). Not taken: a reader outside turns (its own state store and a way to send, for one small file).
2. **Asked for by the owner, per container.** `Start { to, messageType, progressType }`; `progressType` is checked like
   `messageType` (not empty, at most 200 characters, no NUL) and kept in `EndTarget`, the first `Start` winning as
   for the end. With it, the actor sets `CICADA_PROGRESS_FILE` to the job's `ContainerJob::PROGRESS_FILE` (a constant,
   default `/tmp/cicada-progress`) in the spec, after the job's `spec`, so a job cannot override it; at the create the
   path is checked with the path rule of `copy_in`, and a refused path ends the container `failed` with the kind
   `notStarted` and nothing is made, as a refused input does. The rustdoc warns that `copy_out` cannot read a tmpfs:
   with a read-only root and a tmpfs on `/tmp`, set `PROGRESS_FILE` on a volume. Without `progressType` nothing is
   read and no env is set. `ContainerMsg::start_with_progress` builds it.
3. **Read at each running poll and once at the end.** `copy_out(name, path, CopyLimits { max_bytes: 64 KiB,
   max_files: 1 })`, at each poll of a running container and once in the turn that records an end for which the
   container ran: `exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped` and `timedOut`; not `missing`,
   `notStarted` or `runtimeErrors`. A missing file, a skipped one (too big, not a regular file) and a failed read
   send nothing and do not count as runtime errors: progress never ends a container. The text is lossy UTF-8 with
   NUL replaced by U+FFFD (the helper of `actor/text.rs` the logs use). An empty text is sent only after a non-empty
   one. When its hash differs from `progress_hash`, `seq` goes up by one, the hash is kept, and the actor sends
   `ContainerProgress { key, seq, text }` with the end's retry policy to `to` as `progressType`; `seq` and the hash
   move on also when the send is refused, which is not retried and does not block the end.
4. **Order: sent before the end, not always handled before it.** The last progress is sent in the end's commit,
   before the end message. A progress message the owner fails to handle is retried after a backoff, so the end can
   overtake it (G5); the rustdoc and the handoff tell owners to drop a progress message whose `seq` is not above the
   last they handled, and any after the end.
5. **The program writes atomically, as plain bounded text.** Write to a temp name in the same folder and rename, as
   `copy_out` already documents; a half-written file is sent as it is and corrected by the next poll. The text is sent
   as it is (no parsing), at most 64 KiB; a longer file is skipped by the limit.

## Tests

With `FakeRuntime` (files put with `put_file`) and `TestActors` on virtual time: three changes over polls give `seq` 1,
2, 3 and an unchanged file sends nothing; the last change before the exit is sent in the end's commit before the end;
no `progressType` reads nothing and sets no env; a missing file, a file over 64 KiB and a failing `copy_out` send
nothing and leave `errors` at 0; a NUL becomes U+FFFD; an empty first file sends nothing; a refused send moves `seq`;
a bad `progressType` is refused; a refused path ends the container `notStarted`; the env is set after the job's spec;
ends without a run read nothing; a contract snapshot of the test job's new contract.

## Docs

The contract of every container job changes: the handoff asks for the 0.23.0 upgrade section (contracts and the Rust
fields of `Start`, `EndTarget` and `ContainerState`), the guide's progress section with the env variable, atomic writes,
the tmpfs warning and dropping late progress by `seq`. `docs/reference/contracts` has no
container contract (the job is the host's).
