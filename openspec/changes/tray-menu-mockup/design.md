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
   and a badge for the text on the right (`NSMenuItemBadge` with a string, macOS 14.0). The image is the board's icon in
   a 26 pt circle with the board's fill (green for running and done, red for failed, abandoned and rescheduled, orange
   for finishing, blue filled for the selected context, grey otherwise), drawn as non-template images, one per kind and
   appearance (light and dark), kept and swapped when the appearance changes. The menu asks AppKit to show item images
   where the system hides them by default (macOS 27). A row's identity (context name, turn) is its represented object,
   never its title, so joining the second line into the title on older macOS changes nothing it does. The header stays
   the one custom view and gains the disconnecting bar (decision 5). Native items keep keyboard, VoiceOver and the
   system highlight.
2. **Connected (board Main).** Header: the app's title, "Connected to <context>", the switch on. "Running" with "N of M"
   on the right (M: the started worker's concurrency from its config, a new field `concurrency` of `WorkerReady`); rows:
   actor, "<message type> · attempt N", elapsed as m:ss on the right. "Context" rows: name, the server's host on the
   second line. Registered Types with "K of T" on the right, Recent Turns with "N failed" on the right (none when 0).
   Then Open Inspector… (⌘I), Settings… (⌘,), Quit <app> (⌘Q).
3. **Not connected (board Disconnected).** Header: "Not connected", the switch off. After a failed connection, a row
   with the red icon: "Couldn't reach <context>" over "<error> at HH:MM" (the tray records when it failed; the error is
   display text: credentials, URL queries and control characters removed, cut at 200 characters). No Running section and
   no submenus. With no contexts at all the header says "Not connected · Run cicada config" and there is no Connect row.
   The Context rows: picking one selects it (the blue circle and the target of Connect) without connecting, also after a
   failure; the pick lives in `MenuControl`, starts from the config file's current context, survives the reload of the
   contexts when the menu opens while that name still exists, and is never written to the file (the CLI's current
   context stays); then "Connect to <selected>" (⌘K), which, like the switch, connects to the selected context; then
   Open Inspector… (with the `inspector` feature), Settings…, Quit <app>.
4. **Submenus (boards Running and Registered).** Registered Types: "Actor Types" header; a checkmark for each type the
   worker runs, the name, on the right "N running" or "Not on this worker"; "Background Tasks" with "Not available over
   HTTP"; "Choose Types…" opens Settings. Recent Turns: "Finished on this worker" header; each row an icon, the actor, a
   second line and the finish time on the right: done (green check, "<type> · done in m:ss"), failed (red cross, "<type>
   · <error, first line, cut at 80>"), rescheduled after a failure (red cross as on board Running, "<type> · <error>,
   retry HH:MM", the retry's time, or the finish time plus a retry delay, or "retry now"), cancelled (grey, "<type> ·
   cancelled"), abandoned (red cross, "<type> · abandoned"); "Kept until the app quits" at the end. The "N failed" on
   the Recent Turns row counts failed, abandoned and rescheduled turns, as board Main counts the retried one.
5. **Disconnecting (board Disconnecting).** Header: "Disconnecting from <context>…", the switch dimmed. Below it, once
   the worker was ready and while the grace period runs, a bar at elapsed / grace (the worker's `grace_period`, from
   when stopping began) with "Taking no new turns" and "N s left" (the grace left, not the cancel timeout); once the
   grace ends, after Stop Now, or with a grace of zero, the bar is full and the text is "Cancelling turns…". "Finishing"
   with the count and the running rows with the orange clock icon; Stop Now, offered only when the worker was ready, as
   today (a stop before the worker was ready shows no bar and no Stop Now), then the help "Stopping now cancels these
   turns. Their messages stay and run again later." as a disabled row, then Quit <app>. No Context section, submenus,
   Inspector or Settings while disconnecting. Turns still listed when the tray goes offline after Stop Now stay as the
   existing requirement "Turns after Stop Now" says.
6. **The server's host.** `cicada_app::Contexts` gains `host(name) -> Option<String>`: the host and port of the
   context's server URL, never its user, password, path or query. The tray shows it under the context's name.
7. **The status item.** A template image drawn from the boards' hexagon path: a filled dot when connected, a hollow
   one at 55% opacity otherwise, instead of `bolt.fill`.
8. **Where it lives.** The portable menu model (`menu.rs`) holds every text, count and row kind, tested on every
   platform; `appkit/view.rs` draws them. `cicada-app` gains `Contexts::host` and `WorkerReady::concurrency` (additive;
   `WorkerReady` is `#[non_exhaustive]`). `Contexts` keeps only each parsed host and port, never the URL. The new tray
   data (when stopping began, the grace period, whether Stop Now was used, when a connection failed) stays crate-private
   beside `TrayState`, whose public variants do not change.

## Departures from the board

For the owner to agree:

1. **Right-side text as a badge.** Native menus draw text on the right only as a badge, a small capsule, not plain
   grey text as the boards show.
2. **macOS 14.0 to 14.3** have no subtitles: the second line is joined to the title with " · ".
3. **Native sizing and highlight.** Rows take the system's heights and the full blue highlight on hover; the boards'
   38 px rich rows with a subtle grey hover are not reproduced exactly.
4. **The bar** uses the system's progress style inside the header view.
5. **"tried N times"** (board Disconnected) is not shown: the tray does not count connection attempts.
6. **Open Inspector…** shows only with the `inspector` feature of `cicada-tray`.
7. **The status item while disconnecting** is the hollow, dimmed hexagon, as when not connected.
8. **Menu, connecting** has no board (Screens).

## Tests

The menu model on every platform: each state's sections, order, texts and counts (connected, not connected after a
failure, disconnecting with the bar's fraction and seconds, connecting); the submenus' rows and right-side texts;
`Contexts::host` without credentials, path or query; the failure row's text without credentials or queries; a pick while
not connected selecting without connecting; the bar with zero grace and after Stop Now; each recent outcome's row;
`concurrency` carried into "N of M". The AppKit view gets the manual check list, starting with "next to each board,
light and dark".

## Docs

The handoff gives the app guide's menu bar section the new rows, the not-connected and disconnecting menus, and the
host under each context; `WorkerReady::concurrency` and `Contexts::host` in the changelog's 0.26.0 section.
