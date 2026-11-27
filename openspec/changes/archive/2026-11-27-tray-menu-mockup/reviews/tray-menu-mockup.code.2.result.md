Verdict: fix needed

1. `crates/cicada-tray/src/menu.rs:237` and `:260` still leak secrets. `https://host?token=abc@secret` displays as `https://secret`, and `https://user:sec)ret@host/path?token=hidden` displays unchanged. Respect URL component boundaries and redact before truncating.
2. `crates/cicada-app/src/config.rs:469` invents port 443 for `https://host:` and drops port 8443 from `https:///host:8443/`, which reqwest accepts. Preserve parsed nondefault ports and require explicit port digits for defaults.
3. `crates/cicada-tray/src/appkit/view.rs:284` leaves existing items in the menu while taking their cache. A synchronous observer that reenters during insertion sees an empty cache and inserts duplicates. The outer render overwrites the nested cache, leaving untracked rows. Defer nested renders until the active render restores its state.
4. `crates/cicada-tray/src/appkit/view.rs:404` cuts context hosts at 210 points. Main's badge-free context row allows 242 pixels after padding, icon and gap, so a 225-point host fits the board but gets ellipsized. Derive the budget from the actual row space.

A headless probe reproduced the URL findings. Cargo suite, native badge drawing, VoiceOver and older macOS were unchecked.
