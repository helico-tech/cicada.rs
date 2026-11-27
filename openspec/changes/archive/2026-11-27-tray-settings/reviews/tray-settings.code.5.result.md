Verdict: fix needed

1. `config.rs:30`, `operator.rs:43`, `settings.rs:498`: these readers bypass suffix validation. With `CICADA_CONFIG` pointing to `link -> missing.toml/` (or `missing.toml/.`), `Config::load` treats ENOENT as empty config. I verified `config get-contexts` rejects both while `--server ftp://localhost get actors` proceeds to HTTP request construction. Share the suffix check across readers and test the dangling links.

The new `ContextFile` regression passed with all 44 filtered tests. Full gate and interactive AppKit were not run.
