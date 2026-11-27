Verdict: pass

Blocking findings

None. All five tasks are ticked and their work is in the diff. Each spec scenario has a test: the second message's
schema formatted alone (`types_formats_only_the_visible_choice...`), a filter hiding every watch with no page asked by
scroll and Load more offered, Show Actor to the exact watcher and Back to the watch, and both failures of the error
scenario (Types 503 leaves no stale list; a Watches page fails under the "absent" filter, then Retry from the same
cursor). Types reads once per visit, Refresh or context. Refresh clears the selection and the schema. A superseded
formatting is dropped. Watches pass the cursor back unchanged and resume on scroll when the filter is cleared. Show
Actor is offered only for a `WatchTarget::Actor` target. Every `unsafe` has a SAFETY reason. Watch borrows are
temporaries released before `send_*` and AppKit calls. The tests use 10 s wall-clock bounds. No protected docs are
edited, and every added line is 120 columns or less. The versions are 0.26.0.

Notes

1. The design's Tests list "each page keeping its own filter text". It has no test. It holds by structure: each model
   has its own `prefix`, and `sync_search` reads it. It is not a spec scenario.
2. `types.rs` tests check "no request" (second `show()`, after `use_client`) only with an immediate `try_recv`. That
   cannot catch a late request. Asserting `paging == Idle`, as the Watches test does, would be stronger.
3. `Watches::show` reads the first page on a first visit even while a filter is typed. This is sensible and is in the
   rustdoc. The handoff could say it in the Caveats.
4. CHANGELOG also rewords the existing Messages entry ("Messages shares the Actors list's pager"), which is out of
   scope but harmless.
5. The Watches footer also offers Load more with no filter. This is harmless and not in the design.

Not checked

I ran `cargo test -p cicada-inspector` in the builder's worktree: 54 tests and 1 doctest passed. I did not run clippy,
fmt, the workspace tests, OpenSpec validate or the gate. I did not run the GUI by hand: the window, light and dark
mode, ⌘R, keyboard and VoiceOver. The handoff honestly reports these as "ran by code, not seen", with ⌘R and VoiceOver
still to do.
