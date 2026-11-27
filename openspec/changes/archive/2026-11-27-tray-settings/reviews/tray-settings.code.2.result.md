Verdict: fix needed

1. `config.rs:107–118,295–307` — The resolver follows only final-component symlinks. With `CICADA_CONFIG=current/config.toml`, I retargeted the directory symlink while set-context waited on the old lock. It wrote to the new directory while that directory’s lock was held. Resolve symlinked parents before and after locking and use the pinned path for reads and saves.
2. `settings.rs:230–232,259–262` — Open two forms before a worker entry exists. Save concurrency 9 in one, then change only types in the other. The second Apply resets concurrency to 16. Under the tray lock, initialize defaults only if the entry is still absent and merge actual edits otherwise.
3. `appkit.rs:280–287,408–412` — Registration can change between creating the switches and loading the form. If a type appears between those reads, Apply can drop it because it has no switch. Use one registration snapshot for both.

Refusing writes on lockless mounts is acceptable. The other round-one items are addressed, including the handoff and changelog. Config and tray tests passed. Interactive AppKit checks and the full gate were not run.
