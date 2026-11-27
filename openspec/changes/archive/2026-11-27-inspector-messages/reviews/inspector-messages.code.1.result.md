Verdict: fix needed

1. `model.rs:374,463`. Show Actor history keeps only the destination address. Jump from Messages A, revisit Messages with prefix and selection B, jump again, then go Back three times. The first Messages entry shows B. Forward after a context switch can also select a same-named actor on a different server. Store and restore each jump's source and bind replay to its context.
2. `appkit/messages.rs:162,243`. Select a message with a long header value or many headers. The unscrollable envelope pushes Show Actor and the payload below the pane. Keep all three reachable in a fixed-height window.
3. `messages.rs:174,287`. Select a message, let it finish, then search a different prefix that still matches its address. Even an exhausted empty result leaves the old payload and Show Actor visible with no selected row. Reconcile the pane with the new generation when the ID is absent.
4. `appkit.rs:304,519`. Resize renders the footer with a live `RefCell` borrow, and search holds the navigation borrow while rendering either table. Copy the needed values before AppKit can reenter.
5. `tests/messages.rs:2,144–174`. The new tests use real deadlines and debounce. The stale-format test aborts the first task before it can run on the current-thread executor, then checks only the row ID after joining. Control timing and release an already-started stale format after the second selection, then assert the payload stays current.

All 45 inspector tests and one doctest passed, including the 39 existing tests. Actors tests were unchanged. No added line or checked Cargo.toml line exceeded 120 columns. No protected docs changed. The handoff labels scripted checks “run by code, not seen.” Visual AppKit checks, workspace checks and gate were not rerun.
