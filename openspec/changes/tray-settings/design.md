# Design

The tray connects to the current context of `cicada config`'s file and runs every registered type with the default
concurrency (`tray-menu-bar`). Users need to configure it in a native Settings window: pick or add a context, set its
server and token, choose the types this worker runs, and how many turns run at once (owner's proposal and mockup).

## Decisions

1. **`cicada-app` shares its config code.** A public `cicada_app::ContextFile`: `load(&Env)` reads the file `cicada
   config` uses (the same path rules and parser), `names()`, `current()`, `context(name) -> Option<ContextEntry>`
   (`server`, `token`, `headers`), `set_context(name, entry)` (replaces the whole context, and makes it current when no
   context is, as `set-context` does today), `set_server_and_token(name, server, token)` (keeps the context's headers,
   or creates it with none), `use_context(name)` and `save()`, which writes through the existing atomic save (mode
   0600). `ContextEntry`'s `Debug`, and that of the shared `Config` and `Context` types, prints the token, header values
   and a password in the server URL as `***`, as `Target` does. Parse errors are sanitized in the shared loader (a TOML
   diagnostic can quote a token; the sanitizing `Contexts::load` does today moves there), so every caller, the worker
   included, gets the safe text. The `config` operator commands are rewritten on it; their output and errors stay as
   they are. `Contexts` stays for the menu.
2. **One writer at a time.** `ContextFile::update(&Env, f)` takes an exclusive lock on a lock file next to the config
   file's resolved target (`<target>.lock`, so a symlink and its target share one lock; the `flock` way the placement
   identity uses), re-reads the file, applies `f`, saves, and unlocks. The operator commands that write (`use-context`,
   `set-context`) and the tray use it, so neither loses the other's change. Readers do not lock.
3. **The tray's own file.** `tray.toml` in the directory of the config path as given (a relative `CICADA_CONFIG` is
   taken against the working directory; a symlinked config file keeps `tray.toml` next to the link). A config path whose
   name, or whose resolved target, is that `tray.toml` or its lock file `tray.toml.lock` is refused, so no config write
   can replace either. `[contexts.<name>]` holds `types = [...]` and `concurrency = N`, written atomically, mode 0644
   (no secrets), under its own lock (`tray.toml.lock`, by the tray file's path), since two config files in one directory
   share one `tray.toml`. A context without an entry inherits: no `--types`, no `--concurrency`.
4. **The worker's arguments.** With a saved entry, `connect(context)` always passes `--types` with the saved names still
   registered and `--concurrency N`, so a choice made in Settings, all types included, overrides `CICADA_TYPES` and
   `CICADA_CONCURRENCY`. The saved names are first filtered to the registered ones. If what is left is every registered
   type, the tray passes `--types all` (also when the only registered type is literally named `all` or `none`: `all`
   then runs exactly that type); if it is a single type literally named `all` or `none` while other types are registered
   (the keywords of `--types` would read it as all, or as none), `connect` ends `Failed` with "the saved type selection
   of <context> cannot be passed to --types; open Settings", and validation refuses picking that selection in the first
   place; otherwise it passes the comma-separated names, which `--types` reads literally. Saved types of which none is
   registered any more, or no type registered at all: `connect` ends `Failed` with "no registered type selected for
   <context>; open Settings". The server and token follow the command line's order (`CICADA_SERVER` and `CICADA_TOKEN`
   in the tray's environment win over the context); the window says so under the server field when either is set.
5. **A portable form model.** Module `settings` (every platform): `SettingsForm` loads both files for a context, holds
   the edited values, validates them (the server by the URL check the worker over HTTP uses, moved to one shared
   function; at least one type; a concurrency from 1 through 1,000; a new context's name of 1 to 64 characters from `A-Z
   a-z 0-9 . _ -`, while existing names of any form are kept), reverts, and applies, holding the lock of decision 2 for
   both writes (the config lock, then the tray file's lock, always in that order) so two applies never interleave:
   `ContextFile::update` changes only the fields the user edited (server, token, a new context, `current-context`),
   refusing an Add whose name exists by then, then `tray.toml` is written. If the second write fails, Apply reports
   "connection saved; worker settings not saved: <error>", keeps the window open with the values, and does not
   reconnect; the form marks the connection fields saved (an added context becomes an existing one, so a second Apply
   does not meet the duplicate-name check), and a second Apply writes only what still differs, keeping edits another
   writer made meanwhile, and reconnects once both are saved. Its unit tests run in the gate everywhere.
6. **Apply and reconnect.** The tray's control gains `apply(context)`: while the tray holds a connection (any context)
   it disconnects, waits for no connection and connects to `context`, replacing a pending pick; with no connection it
   only clears a pending pick; during a quit it does nothing (Quit wins). Not taken: reusing `pick`, which ignores a
   pick of the connected context.
7. **The window.** One `NSWindow` (`appkit` module), opened by Settings… (⌘,; enabled now) and brought to the front and
   made key when open; ⌘W and Esc close it, and it reopens with the values from disk. Grouped boxes in the System
   Settings style: Connection (a context pop-up with the names, a separator and "Add…", which asks for a name in a sheet
   that can be cancelled; the server; the token in an `NSSecureTextField`), Worker (an `NSSwitch` per registered type;
   the concurrency in a field with a stepper from 1 to 1,000). "Revert" and "Apply and Reconnect". A validation error
   shows under its field and keeps the window open.
8. **Secrets.** The token is shown masked and written only into the config file, as `cicada config` does; it is never
   logged, never in `Debug`, never in the tray file. The Keychain stays out of scope.

## Tests

`ContextFile`: load, set, use and save round trips; headers kept; a missing file; a parse error without the token in its
text; `Debug` without the token; two `update` calls from two threads both kept; the operator commands unchanged (their
existing tests). Tray file: read, write, a missing file, a relative and a symlinked config path, a config file named
`tray.toml` refused. Worker arguments: no entry adds nothing; an entry adds `--types` and `--concurrency`, also over
`CICADA_TYPES` and `CICADA_CONCURRENCY`; no registered type left fails the connect; saved `[all, old]` against
registered `[all, new]` fails the connect, not `--types all`; two config files in one directory both keep their
`tray.toml` entries. `SettingsForm`: each validation error; an existing odd name kept; revert; apply keeps a context
another writer added meanwhile and sets `current-context`; a failed `tray.toml` write reported, and a retry after an Add
keeps a `cicada config` edit made meanwhile. `apply`: while online with the same and with another context, offline with
a pending pick, during a quit. The window gets a manual check list in the tasks (activation and focus, shortcuts, sheet
cancel, reopen).

## Docs

The handoff gives the app guide's menu bar section the Settings window and `tray.toml`, and the operations or command
line docs the shared config file; the changelog's 0.26.0 section names `ContextFile`, the window and `tray.toml`.
