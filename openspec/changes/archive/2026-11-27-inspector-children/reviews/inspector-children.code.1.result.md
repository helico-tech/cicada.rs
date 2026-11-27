Verdict: pass

Blocking findings: none. All five tasks are done in the diff. Every spec scenario and every test in the design has a
model test (`src/tests/actors/children.rs`). The 39 `cicada-inspector` tests pass (I ran them myself). Each wait has a
wall-clock bound, and the held answers are ordered by `Notify`, so the tests do not depend on machine speed. Checked
and correct:
- paths as identities, and the selection as a path;
- one `ChildRead` marker per parent, with the cursor passed back unchanged;
- no duplicate children after more…×2 or open/close/reopen;
- stale reads dropped by generation, then by the `child_reads` entry;
- Retry, "No children." and "Not found.";
- "—" outside the prefix;
- the fold rules, including typing a letter and deleting it;
- `previous` items dropped only after `reloadData`, and re-open under `syncing`;
- `wants_more` counting root rows;
- borrows released before AppKit calls, and a reason on every `unsafe`;
- CHANGELOG 0.26.0, and a handoff that matches the code. No protected docs were edited.

Notes
1. Lines over 120 columns that rustfmt does not catch: `crates/cicada-inspector/Cargo.toml` (the two feature lines,
   133 and 134), the `wants_more` doc comment in `actors.rs` (122), and the Caveats line in `docs-handoff.md` (122).
2. The "native smoke run" is not a committed test. The builder wrote a throwaway example
   (`examples/outline_smoke.rs`, which pulled in the real `appkit.rs` by `#[path]` plus a `check.rs`), ran it once
   against a fake server, and deleted it. Only `target/outline-smoke/run.log` is left: 7 PASS lines and 1 PARTIAL
   (⌘R). It drove a real window by code (performClick and so on). Nobody saw that window, and its assertions can no
   longer be read. Record those items as "run by code, not seen", not as done. ⌘R, keyboard, VoiceOver, light and
   dark, and every visual item still need the owner's manual pass. The handoff's Status line should say the script
   was uncommitted.
3. `last_visible_root` returns None when only the children of the last root are in view, so root paging waits until
   a root row scrolls into view.
4. The ⌘R scenario (a deleted row on reload) is tested through the search reload (`reload_keeps_later_root_folds…`).
   `refresh()` itself is covered only by the other two tests, so it shares that code path but has no test of its own.

Not checked: the window by eye; how AppKit keeps items under a collapsed parent after their map entries are dropped;
the gate evidence; the full workspace tests.
