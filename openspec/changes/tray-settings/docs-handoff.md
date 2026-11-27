# Docs handoff for tray-settings

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The tray's Settings… item (⌘,) opens a native Settings window. Connection: a context pop-up with the config file's
contexts, a separator and Add…, which asks for a name in a sheet that can be cancelled; the server; the token in a
secure (masked) field. Worker: a switch per registered type (past eight types the list scrolls), and the turns at
once in a field with a stepper from 1 to 1,000. Revert and Apply and Reconnect. A validation error shows under its
field and nothing is written. Esc and ⌘W close the window; it reopens with the values on disk, and with the switches
of the types the app registers by then. While open, Settings… brings it to the front. Cancelling Add… keeps the
unsaved edits in the fields.

Apply writes only what the user changed: the server, the token, a new context, and `current-context` when another
context is selected, into the config file, through the same locked code as `cicada config`; other contexts and fields
another writer changed meanwhile stay as they are on disk. Then the worker settings go into `tray.toml`: only the
field the user changed (types or turns at once), keeping the other as another writer saved it, or, for a context
without an entry, the shown types and turns at once as its new entry; if another writer saved an entry meanwhile,
only the edited fields go into it. If that second write fails, the window says
"connection saved; worker settings not saved: <error>" when the config file was written, or "worker settings not
saved: <error>" when only the worker settings changed. It keeps the values and does not reconnect; a second Apply
writes what still differs. After a full save, a tray with a
connection (connecting, online or stopping, to any context) disconnects, waits for no connection, and connects to the
applied context with the new arguments. An offline tray stays offline.

`tray.toml` sits in the folder of the config path as given (a relative `CICADA_CONFIG` against the working directory;
next to a symlink, not its target). `[contexts.<name>]` holds `types = [...]` and `concurrency = N`; mode 0644, no
secrets. A context with an entry connects with `--types` (the saved types the binary still registers, or `all` when
that is every registered type) and `--concurrency`, which win over `CICADA_TYPES` and `CICADA_CONCURRENCY`. A context
without an entry inherits those variables and the defaults; the window then shows every type on and the worker's
default of 16, and Apply saves those as the context's entry. An app with no registered type, a saved selection with
no registered type left, or a lone type named `all` or `none` while others are registered, ends the connection
`Failed` with an error that says to open Settings. `CICADA_SERVER` and `CICADA_TOKEN`
in the tray's environment still win over the context, and the window says so under the server field.

`cicada config` and the tray refuse a config path whose name, or whose symlink target's name, is `tray.toml` or ends
in `.lock`, in any letter case (a case-insensitive file system treats `TRAY.TOML` as `tray.toml`), so no config write
replaces a lock file or the tray's file. Both files' parse errors name the line, never the text. Every write takes an
OS file lock; on a file system without file locks the write fails with "cannot lock <file>; its file system must
support file locks" instead of risking another writer's change.

`cicada_app::ContextFile` is the config file of `cicada config`: `load(&Env)`, `update(&Env, f)`, `path`,
`resolved_path`, `names`, `current`, `context` (a `ContextEntry`: `server`, `token`, `headers`), `set_context`,
`set_server_and_token`, `use_context` and `save`. `update` holds an exclusive lock on `<target>.lock` next to the file
a save writes (the path with every symlink followed, its folders' included), resolves the whole path again after
locking (and locks anew when it names another file by then), re-reads the locked file, applies `f` and saves that
same file, by its symlink-free path, when it changed. `cicada config use-context` and `set-context`
now go through it and create that lock file. `cicada_app::check_server_url` is the worker's URL check. An unreadable
config file is now reported by every command, the worker included, as `cannot read <path>: not a valid config file at
line N`, without the parser's text. `ContextEntry`'s `Debug` hides the token, header values and a URL password.

Tests: `crates/cicada-app/src/config.rs` (`two_updates_from_two_threads_are_both_kept`,
`a_parse_error_names_the_line_but_not_the_token_for_every_reader`,
`the_debug_of_a_context_and_the_config_hides_credentials`, `a_context_file_sets_uses_and_saves_and_keeps_headers`,
`an_update_through_a_symlink_locks_next_to_its_target_and_keeps_the_link`); `crates/cicada-tray/src/tray_file.rs`
(location, refusal, both entries kept, the argument rules); `crates/cicada-tray/src/settings.rs` (the form);
`crates/cicada-tray/src/menu.rs` (`apply_…`); `crates/cicada-tray/tests/control.rs` (`saved_settings_…`). The window
itself has a manual check list.

No guarantee row, wire format, schema or command line flag changes. The CLI reference is unchanged.

## 1. The Settings window

**Where.** `docs/guide/app.md`, the menu bar section of `tray-menu-bar`'s handoff (after `A worker over HTTP`).
**Text.** Settings… (⌘, while the menu is open) opens the Settings window. Pick a context, or Add… a new one (1 to 64
characters from `A-Z a-z 0-9 . _ -`), set its server and token, switch the types this worker runs, and set the turns
at once. Apply and Reconnect saves and, while the tray is connected, reconnects with the new settings. Revert shows
the saved values again. Esc or ⌘W closes the window without saving.
**Caveats.** The token is stored in the config file, as `cicada config` stores it, not in the Keychain. `CICADA_SERVER`
and `CICADA_TOKEN` in the tray's environment win over the window's server and token. Apply while offline saves but does
not connect. When Apply saves the connection but not the worker settings, the window says "connection saved; worker
settings not saved: <error>"; when only the worker settings changed, "worker settings not saved: <error>". Apply
again once the cause is fixed.

## 2. `tray.toml`

**Where.** `docs/guide/app.md`, the same menu bar section.
**Text.** The tray keeps per-context worker settings in `tray.toml` next to the config file:

```toml
[contexts.prod]
types = ["checkout"]
concurrency = 4
```

A context listed there runs only those types (or all, when all are picked) with that concurrency, also when
`CICADA_TYPES` or `CICADA_CONCURRENCY` is set. A context not listed uses those variables and the defaults, until an
Apply in the Settings window saves its entry. If none of
the saved types is registered any more, connecting fails and says to open Settings.
**Caveats.** Two config files in one folder share one `tray.toml`. Do not point `CICADA_CONFIG` at a file named
`tray.toml` or ending in `.lock`, in any letter case; the tray and `cicada config` refuse it.

## 3. The config file is shared and locked

**Where.** `docs/guide/app.md`, `Server and credentials`, after "`config set-context` replaces a context; `config
use-context` switches it."
**Text.** `config set-context` and `config use-context` take a lock on `<file>.lock` next to the config file (next to
its target when it is a symlink) while they write, so they and the menu bar app keep each other's changes. An
unreadable file is reported with its path and line, not its contents. A config path named `tray.toml` or ending in
`.lock`, in any letter case, is refused.
**Caveats.** The lock covers one machine. Readers take no lock. On a file system without file locks (some network
mounts), the write fails; point `CICADA_CONFIG` at a local file.

## 4. `ContextFile` for hosts

**Where.** `docs/guide/app.md`, near the existing mention of `Contexts`, if the guide lists the app's public items.
**Text.** `cicada_app::ContextFile` reads and writes the config file with the code of `cicada config`. Use
`ContextFile::update(&env, |file| { ...; Ok(()) })` for a write that may run next to another; it re-reads the file under
the lock and saves it when changed.
**Caveats.** `set_server_and_token` keeps the context's headers and leaves `current-context` alone; `set_context`
replaces the whole context and makes it current when none is.

## Blocking

None.

## Status

Ran `cargo test -p cicada-app`, `cargo test -p cicada-tray` and `cargo test -p cicada-examples --test docs` after
code review 2, and `--doc` after code review 1; all pass. All text above is proposed; no protected prose was edited.
The manual check list of the window is reported in the build report.
