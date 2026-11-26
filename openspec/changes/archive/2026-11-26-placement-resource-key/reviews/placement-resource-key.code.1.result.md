Verdict: fix needed

## Blocking findings

1. `crates/cicada-actors/src/placement/member.rs` `lock()` (commit bd1e0e5b): on `PermissionDenied`/`ReadOnlyFilesystem` it goes on without a lock also when the lock file exists and can be read. Before, that case failed with `Invalid`. Now a second process runs under the same identity on the same machine, which is what the lock is for. Example: the lock file was made by another user (an old root container on a shared volume) who still holds it. How to see it: `a_read_only_folder_loads_the_identity_without_a_lock` (case `missing = false`) asserts that a second load succeeds while `read` is alive. `File::try_lock` uses `flock` on Unix, and an exclusive `flock` works on a read-only fd: I checked this on macOS with a 0444 file, and a second fd got `EWOULDBLOCK`. Fixed: on a permission error, open the existing file read-only and `try_lock` it (`Locked` when held). Go on without a lock only when the file cannot be opened at all (missing in a read-only folder). Fix the test, rustdoc, CHANGELOG and handoff §5 to match.

## Notes

1. Scope: bd1e0e5b (the identity fixes) is not in this change's design or tasks. It comes from placement-identity-lock's review notes. The handoff names it, which is fine, but the tasks could have said so.
2. A node that takes the key keeps its own `contested` entry until it expires, so the state shows it as both holder and contester for up to one silence time. Harmless, but consider removing that node's contest of the key when it takes it.
3. Three doc-comment lines go over 120 columns (rustfmt does not wrap comments): `placement.rs` (the arm paragraph of `PlacementActor`), `member.rs` (`Offer { node, instance, labels, places, resource }`), and the CHANGELOG intro line naming `Capacity`.
4. A draining or report-marked holder still holds the key, so a shared daemon then gets no new places from either node. This follows the design, but the guide could say so.

## Not checked

I ran no test suite, gate or E2E (the builder reports a passing workspace, clippy, fmt and openspec run; the gate is pending on main). I did not check Docker `/info` across versions, or the Windows lock behaviour for the fix above.
