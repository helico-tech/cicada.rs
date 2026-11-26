# Design

The tray connects to the current context of `cicada config`'s file and runs every registered type with the default
concurrency (`tray-menu-bar`). Users need to configure it in a native Settings window: pick or add a context, set its
server and token, choose the types this worker runs, and how many turns run at once (owner's proposal and mockup).

## Decisions

1. **`cicada-app` shares its config code.** A public `cicada_app::ContextFile`: `load(&Env)` reads the file `cicada
   config` uses (the same path rules and parser), `context(name) -> Option<ContextEntry { server, token, headers }>`,
   `set_context(name, entry)`, `use_context(name)` and `save()`, which writes through the existing atomic save (mode
   0600). The `config` operator commands are rewritten on it, so the command line and the tray share one code path.
   `Contexts` (names and current, from `tray-menu-bar`) stays for the menu. A context name follows the command line's
   rule (whatever `set-context` accepts today; the builder names it in the rustdoc).
2. **The tray's own file.** `tray.toml` in the config file's directory (so `CICADA_CONFIG` moves it too):
   `[contexts.<name>]` with `types = [...]` and `concurrency = N`. A missing file or context means every registered
   type and the worker's default concurrency. A type no longer registered is dropped when read. Written atomically,
   mode 0644 (no secrets).
3. **The worker's arguments.** `connect(context)` adds `--types a,b` when the context's settings pick fewer than all
   registered types, and `--concurrency N` when one is set; nothing otherwise, so the worker's own defaults and the
   `CICADA_…` variables keep their meaning.
4. **A portable form model.** Module `settings` (every platform): `SettingsForm` loads both files for a context, holds
   the edited values, validates them (a server URL with `http` or `https`, by the same check the worker over HTTP uses;
   at least one type; a concurrency from 1 through the largest the `--concurrency` flag accepts; a new context's name by
   decision 1's rule), reverts, and applies: it re-reads the config file and changes only the edited context and
   `current-context`, so contexts changed meanwhile by `cicada config` are kept, then writes `tray.toml`. Its unit
   tests run in the gate everywhere.
5. **The window.** One `NSWindow` (`appkit` module), opened by Settings… (⌘,; enabled now) and brought to the front when
   open; ⌘W and Esc close it. Grouped boxes in the System Settings style: Connection (a context pop-up with the names, a
   separator and "Add…", which asks for a name in a sheet; the server; the token in an `NSSecureTextField`), Worker (an
   `NSSwitch` per registered type; the concurrency in a field with a stepper). "Revert" and "Apply and Reconnect". A
   validation error shows under its field and keeps the window open. Applying while the tray holds a connection
   disconnects, waits for no connection, and connects to the picked context (the pending pick of `tray-menu-bar`);
   while offline it only saves.
6. **Secrets.** The token is shown masked and written only into the config file, as `cicada config` does; it is never
   logged or put in the tray file. The Keychain stays out of scope.

## Tests

`ContextFile`: load, set, use and save round trips; a missing file; the operator commands unchanged (their existing
tests). Tray file: read, write, a missing file, a dropped type. Worker arguments: all types and no concurrency add
nothing; a subset adds `--types`; a concurrency adds `--concurrency`. `SettingsForm`: each validation error; revert;
apply keeps a context another writer added meanwhile and sets `current-context`; apply while connected asks for a
reconnect and offline does not. The window gets a manual check list in the tasks.

## Docs

The handoff gives the app guide's menu bar section the Settings window and `tray.toml`, and the operations or command
line docs the shared config file; the changelog's 0.26.0 section names `ContextFile`, the window and `tray.toml`.
