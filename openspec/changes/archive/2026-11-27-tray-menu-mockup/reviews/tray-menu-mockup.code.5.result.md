Verdict: fix needed

1. Splitting before redaction leaks credentials, paths, queries and fragments across 22 parser-accepted whitespace characters. For example, `https://user:pa\u{00a0}ss@host/private?token=hidden#frag` displays `<url> ss@host/private?token=hidden#frag`. Redact across whitespace, suppressing the remainder when boundaries are ambiguous. [menu.rs:232](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:232)
2. `https:/\user:pass@host/private` and `https:user:pass@host/private` both parse and display verbatim. Recognize the authority forms accepted by the special-scheme parser. [menu.rs:245](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:245)

The Rust probe covered 87 characters. The test suite and AppKit rendering were unverified.
