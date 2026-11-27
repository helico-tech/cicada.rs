Verdict: fix needed

1. At `crates/cicada-app/src/config.rs:306`, atomically replace the checked regular file with a symlink to `missing.toml/` after `link_target` returns. The open follows the replacement and returns ENOENT, which becomes empty config. Open the checked leaf without following symlinks and revalidate any replacement before reading it.

Verified ENOENT with a filesystem probe. Rust tests and gate were not run.
