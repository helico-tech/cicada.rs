# Design

The owner's mockup (`mockup/`, boards Main, Disconnected, Disconnecting, Running and Registered; `mockup/SOURCE.md`)
shows rich menu rows: an icon in a tinted circle, a title, a second line and text on the right. `tray-menu-bar` drew
plain items instead and said nothing about it; the owner found it on 2026-11-27 ([MOCK-FIX]). This change makes the
menu match the boards, and follows `gui-mockup-boards`.

## Screens

| Screen | Board |
| --- | --- |
| Menu, connected | `Main.dc.html` |
| Menu, Recent Turns submenu open | `Running.dc.html` |
| Menu, Registered Types submenu open | `Registered.dc.html` |
| Menu, not connected (offline or failed) | `Disconnected.dc.html` |
| Menu, disconnecting | `Disconnecting.dc.html` |
| Menu, connecting | no board: as Main with "Connecting to <context>…" and no Running rows yet (owner to agree) |
| Status item | the boards' bar: a hexagon with a dot, filled when connected, dimmed otherwise |

## Decisions

1. **Rich rows with native items.** Every rich row is an `NSMenuItem` with an image, a title, a subtitle (macOS 14.4)
   and a badge for the text on the right (`NSMenuItemBadge` with a string): the image is the board's icon drawn in a
   26 pt tinted circle (green for running and done, red for failed, orange for finishing, blue filled for the current
   context, grey otherwise), made once per kind and kept. The header stays the one custom view and gains the
   disconnecting bar (decision 5). Native items keep keyboard, VoiceOver and the system highlight.
2. **Connected (board Main).** Header: the app's title, "Connected to <context>", the switch on. "Running" with "N of
   M" on the right (M: the worker's concurrency, a new field `concurrency` of `WorkerReady`); rows: actor, "<message
   type> · attempt N", elapsed as m:ss on the right. "Context" rows: name, the server's host on the second line.
   Registered Types with "K of T" on the right, Recent Turns with "N failed" on the right (none when 0). Then Open
   Inspector… (⌘I), Settings… (⌘,), Quit <app> (⌘Q).
3. **Not connected (board Disconnected).** Header: "Not connected", the switch off. After a failed connection, a row
   with the red icon: "Couldn't reach <context>" over "<error> at HH:MM". No Running section and no submenus. The
   Context rows, then "Connect to <context>" (⌘K), Open Inspector…, Settings…, Quit <app>.
4. **Submenus (boards Running and Registered).** Registered Types: "Actor Types" header; a checkmark for each type the
   worker runs, the name, on the right "N running" or "Not on this worker"; "Background Tasks" with "Not available over
   HTTP"; "Choose Types…" opens Settings. Recent Turns: "Finished on this worker" header; each row the done or failed
   icon, the actor, "<message type> · done in m:ss" or the failure's short text, the finish time on the right; "Kept
   until the app quits" at the end.
5. **Disconnecting (board Disconnecting).** Header: "Disconnecting from <context>…", the switch dimmed, below it the
   grace period as a bar with "Taking no new turns" and "N s left" (the tray records when stopping began and the total
   it waits); "Finishing" with the count, the running rows with the orange clock icon; Stop Now with "Stopping now
   cancels these turns. Their messages stay and run again later." as a disabled row; Quit <app>. No Context section,
   submenus, Inspector or Settings while disconnecting.
6. **The server's host.** `cicada_app::Contexts` gains `host(name) -> Option<String>`: the host and port of the
   context's server URL, never its user, password, path or query. The tray shows it under the context's name.
7. **The status item.** A template image drawn from the boards' hexagon path: a filled dot when connected, a hollow
   one at 55% opacity otherwise, instead of `bolt.fill`.
8. **Where it lives.** The portable menu model (`menu.rs`) holds every text, count and row kind, tested on every
   platform; `appkit/view.rs` draws them. `cicada-app` gains `Contexts::host` and `WorkerReady::concurrency`
   (additive; `WorkerReady` is `#[non_exhaustive]`).

## Departures from the board

For the owner to agree:

1. **Right-side text as a badge.** Native menus draw text on the right only as a badge, a small capsule, not plain
   grey text as the boards show.
2. **macOS 14.0 to 14.3** have no subtitles: the second line is joined to the title with " · ".
3. **The bar** is part of the header view, so it sits under the header as on the board, but uses the system's
   progress style.
4. **"tried N times"** (board Disconnected) is not shown: the tray does not count connection attempts; the row shows the
   error and its time.
5. **Menu, connecting** has no board (Screens).

## Tests

The menu model on every platform: each state's sections, order, texts and counts (connected, not connected after a
failure, disconnecting with the bar's fraction and seconds, connecting); the submenus' rows and right-side texts;
`Contexts::host` without credentials, path or query; `concurrency` carried into "N of M". The AppKit view gets the
manual check list, starting with "next to each board, light and dark".

## Docs

The handoff gives the app guide's menu bar section the new rows, the not-connected and disconnecting menus, and the
host under each context; `WorkerReady::concurrency` and `Contexts::host` in the changelog's 0.26.0 section.
