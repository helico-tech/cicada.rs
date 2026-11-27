Verdict: fix needed

1. `crates/cicada-app/src/config.rs:202–210` — Rebuilding the target from `parent()` and `file_name()` drops trailing `/` and `/.`. With an existing regular config, I ran `cicada config set-context` with each suffix in `CICADA_CONFIG`. Both commands succeeded and rewrote the file, although the OS rejects both paths with ENOTDIR. Reject directory-only spellings and test that the file stays untouched.

The folder-retarget tests and my independent two-lock repro passed. The stale-form test passed and preserves both edits under the tray lock. Show and Revert use the switches’ registration snapshot. I verified that wiring by inspection and found no dedicated regression test.

Note: A stale untouched form still rewrites the tray file, with its values preserved.

Config and tray tests passed. Interactive AppKit and the full gate were not run.
