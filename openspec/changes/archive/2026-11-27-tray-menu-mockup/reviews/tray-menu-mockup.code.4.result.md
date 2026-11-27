Verdict: fix needed

1. Splitting on whitespace before redaction exposes URL components after tabs or newlines. `https://user:pa\tss@host/private?token=hidden#frag` displays `<url> ss@host/private?token=hidden#frag`. A tab inside the delimiter (`https:\t//user:pass@host/private`) bypasses redaction entirely. The URL parser accepts both inputs. Handle tabs and line breaks inside URL candidates before splitting so the credentials and suffix are discarded. [menu.rs:230](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:230)

A targeted Rust probe confirmed that mid-word URLs and uppercase schemes redact correctly, and explicit default ports survive parser whitespace handling. Cargo suites and native AppKit rendering were not checked.
