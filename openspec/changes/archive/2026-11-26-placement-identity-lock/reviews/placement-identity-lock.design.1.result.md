Verdict: not yet

## Blocking

1. **The lock is dropped before the node runs.** design.md Decision 2: `PlacementMember::new` (member.rs) keeps only `identity.instance`, so the lock goes when the member is built, before any offer. In the guide's pattern (actors.md, "The member") that happens inside `app()`. Also, `NodeIdentity` derives `Clone, PartialEq, Eq`, and a `File` is none of these. What I would do: decide that the member keeps the whole `NodeIdentity` or its guard. Choose `Arc<File>` (keeps `Clone`) or drop `Clone`, which breaks hosts; the change says "Additive", so name the break if you drop it.
2. **Host-name check vs. containers.** Decision 1: a Docker container's host name is its container id unless `--hostname` is set, and a Kubernetes Deployment pod's is the pod name. With the identity on a volume, every redeploy fails with `OtherMachine`. That is a host break nobody named, and maybe worse than the runtime node-table check that was not taken. Decide which deployments count, or use another machine key. See question Q1.
3. **Error paths without a decision or scenario.** (a) `whoami::hostname()` fails or gives an unusable name (the node table then stores none): fail, or skip the check? (b) `File::try_lock` returns `TryLockError::Error` (for example ENOLCK, or Unsupported on a network folder), not a silent "no lock": fail, or warn and go on? (c) `InvalidIdentity` is a struct with a private `String` today, not an enum. Say what replaces it (an enum marked `#[non_exhaustive]` with an `Invalid` variant?).

## Notes

- Rollback: 0.24.0 reads with `deny_unknown_fields`, and the first load on 0.25.0 writes `host`. A downgrade then stops every node. Name it in the upgrade handoff.
- Take the lock (after `create_dir_all`) before reading or creating the file. Then the "two start at once on a missing file" caveat goes away, and the rustdoc and guide can say so.
- Things that break: the `load_or_create` doctest and `the_identity_is_made_once…` both load twice while `first` lives, which gives `Locked`. The `no temporary file left` test sees the lock file.
- `File::try_lock`: stable since 1.89, and 1.94 is fine. On Unix it uses `flock`, which locks per open file. So the in-process `Locked` test is valid.
- cicada-actors has no `whoami` dependency, and `usable_hostname` is private to cicada. Name the dependency.

## Questions

- Q1: Do Factory or other hosts run nodes in containers with the identity on a volume?

## Not checked

Windows lock behaviour, and the later proposals beyond reading their order.
