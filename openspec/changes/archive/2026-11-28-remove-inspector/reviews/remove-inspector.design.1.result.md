Verdict: not yet

## Blocking

1. **`cicada_app::context_client` is left undecided.** design.md, Decisions 3–4. It is public and unreleased, and its only caller is `cicada-inspector`: the tray does not use it. Its CHANGELOG entry (line 183) and the summary sentence "makes the client for it" (line 26) fall between "inspector entries go" and "tray and `changes` entries stay". Decide keep or remove, as Decision 3 does for `ActorClient::changes`, and say which CHANGELOG lines go.
2. **Boards: the tray menu changes, and its boards are not named.** The tray-menu-mockup boards Main, Disconnected, Registered and Running draw "Open Inspector…". Departure DEP-INSPECTOR ("shows only with the feature") was agreed for that row. Add one sentence: the default build looks the same, DEP-INSPECTOR now means "never shown", so the change needs no board copy (or the owner agrees to that).

## Notes

- Decision 1 names only `appkit.rs`. The code also touches `appkit/view.rs` (it builds the row, sets its target and shows it) and `menu.rs`. In `menu.rs`, `QuitRequest::begin` has a `close_inspector` closure, and three tests are about the inspector. Say whether `begin` loses that parameter.
- Decision 2 is already done in 1889dba8 (ADR 0078, the status of 0076 and 0011, the index rows). Keep it out of the tasks.
- Decision 5: `dependency-trees.sh` lines 51–53 (the `tray-inspector` tree check) and the header comments of both scripts go too.
- Docs: in `docs/guide/app.md`, name the paragraph "Open Inspector" (lines 308–314). It shows `--features cicada-tray/inspector` in a command, not just a link.
- The proposal says "ADR 0011 holds again in full, apart from the tray". The design and ADR 0078 also keep ADR 0056 as an exception. Align the proposal.

## Questions for the owner

- Keep `context_client`?

## Not checked

I built nothing and ran no tests. I did not read the boards' HTML beyond a search for "inspector". The worker-view proposal fits this order.
