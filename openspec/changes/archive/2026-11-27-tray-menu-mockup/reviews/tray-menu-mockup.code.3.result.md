Verdict: fix needed

1. The apostrophe ends redaction inside valid URLs. `https://u:pa'ssword@host/path?token=hidden` displays `<url>'ssword@host/path?token=hidden` and `https://host/#abc'def` displays `https://host'def`. Redact the entire URL before retaining surrounding quotes. [menu.rs:242](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:242)
2. Punctuation trimming removes the closing IPv6 bracket. `(https://[::1]): refused` displays `(<url>]): refused`. Preserve brackets that belong to the host. [menu.rs:245](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-tray/src/menu.rs:245)
3. `"https://host:443 "` parses, but the context displays `host` and drops the explicitly written `443`. Apply the parser's whitespace handling when checking for port digits. [config.rs:477](/Users/avanwieringen/.t3/worktrees/cicada.rs/t3-9c730fe7-26444407a8f15706/crates/cicada-app/src/config.rs:477)

A headless Rust probe reproduced all three. The render guard defers nested renders until caches are restored, and Main's context budget matches the CSS at 242 pt. Cargo suites and native AppKit rendering were unchecked.
