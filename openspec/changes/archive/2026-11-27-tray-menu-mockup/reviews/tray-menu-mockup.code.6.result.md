Verdict: fix needed

1. A `/types` 401 body of `rejected bearer example-token` becomes the terminal `SourceError::Fatal(String)` and reaches the menu. Usage diagnostics can quote values, and a missing cause falls back to the full error. The marker filter also passes `ftp:host?token=example`. Use safe summaries for unclassified causes. [menu.rs:440](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:440) · [turns.rs:80](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-client/src/turns.rs:80)
2. Recent Turns displays the password in `//alice:example-password\n@host` because it checks only the first line. Plain bearer-token errors also pass. Use safe summaries for unclassified turn errors. [menu.rs:346](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:346)

Decisions 3 and 4’s display rules match except for the unknown-cause fallback. The Rust formatting probe confirmed both leaks. Cargo tests and AppKit rendering were unverified.
