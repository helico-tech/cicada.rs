Verdict: fix needed

Paths below are relative to crates/cicada-tray/src unless specified.

1. menu.rs:233 leaks credentials and queries. An HTTP problem mentioning https://user:sec(ret)@host/path?token=abc(def) is displayed in full. Strip complete URL credentials and queries before truncating.
2. menu.rs:358 hides stranded turns when Stop Now reaches Offline before lease release finishes. The spec requires the row to persist until completion. crates/cicada-tray/tests/control.rs:520 checks history only. Cover the offline menu too.
3. appkit/view.rs:248,274,519 and appkit/icons.rs:195 hold mutable borrows across AppKit setters, insertions and image creation. A synchronous menu-change observer that reenters rendering can panic. Release the borrows before calling AppKit.
4. appkit/view.rs:95,237,509 uses a minimum width and sends unbounded text to AppKit. Long actors, hosts or errors can widen the menu instead of ellipsizing. The switch frame is 38×22 versus the boards’ 32×18.
5. menu.rs:320,354 reuses one name for both labels. With app name myapp, the header loses the boards’ “worker” suffix. appkit/view.rs:104,168 also adds two custom views where decision 1 allows only the header. Their text and placement match.
6. menu.rs:390 starts counting at Disconnect. If polling teardown delays the worker’s grace period, the menu says “Cancelling turns…” while grace remains (crates/cicada/src/runtime/worker.rs:497,517).

7. crates/cicada-app/src/config.rs:468 drops an explicit default port. The accessor promises host:port for a URL naming one, but https://host:443 becomes host.

Owner questions

- Do Running and Finishing counts draw on native headers (appkit/view.rs:267)? If absent, approve a fallback. Is the grey cancellation cross the intended glyph?

“Running 0 of 4” follows decision 2. Connecting has no board and worker counts wait for readiness. Offline picks are passive. Contexts stores host and port only. Public changes are additive. New unsafe blocks have reasons.

Unchecked: AppKit rendering, VoiceOver, older macOS, test suite.
