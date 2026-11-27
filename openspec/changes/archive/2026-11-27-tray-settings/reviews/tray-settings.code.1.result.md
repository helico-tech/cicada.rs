Verdict: fix needed

1. `operator.rs:287–296`, `tray_file.rs:46–70` — CLI writes to `CICADA_CONFIG=tray.toml.lock` can replace the held lock inode. On case-insensitive macOS, `TRAY.TOML` also bypasses refusal and can expose config contents in tray parse diagnostics. Reject aliases before loading or locking, including in CLI writes.
2. `config.rs:97–103,164–165,185–189`, `tray_file.rs:129–132` — Retargeting a symlink between locking and saving writes under the wrong lock. Pin the target through read/save. Both writers proceed on Unsupported; report failure when locking is unavailable.
3. `settings.rs:224–254` — One changed worker field replaces both. With two forms open, saving concurrency then types restores stale concurrency. Merge edited fields under the tray lock, including on retry.
4. `settings.rs:102–105,224–227` — With no entry, all types/16 display as selected but Apply keeps inheriting environment overrides. Allow saving those exact values explicitly. The all-types scenario test seeds the file, missing this form path.
5. `tray_file.rs:145` — With no saved entry and zero registered types, connect skips the prescribed failure. Check zero registrations before inheritance.
6. `appkit/settings.rs:277–287,343–346` — Edit a field, then open and cancel Add. Refill discards unsaved inputs. Restore only popup selection.
7. `appkit/settings.rs:85–95` — Thirty types make the non-scrollable window exceed typical displays. Bound its height and scroll the content.
8. `appkit.rs:279–283,405–410` — A later factory call can change registered types; reopening loads a fresh form into the original switches. Rebuild the controls when registrations change.
9. `menu.rs:544–555` — “Apply while connected” tests actions alone. Cover applying a changed selection through to a restarted worker with the new arguments.
10. `docs-handoff.md:16` — Worker-only failure omits “connection saved.” Document both messages. Wrap `CHANGELOG.md:50,56` and `appkit/settings.rs:46` to 120 columns.

macOS compilation, the preseeded all-types integration test and workspace rustfmt passed. Protected paths unchanged. Full suites, gate evidence and interactive AppKit checks were not checked.
