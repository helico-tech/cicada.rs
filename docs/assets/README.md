# Documentation visuals

`tui.png` is the actual Cicada 0.17.1 TUI, captured at 140 columns × 28 rows against the sample service.
The data comes from a temporary SQLite store and the sample's fake sync API. No production data or credentials
are used. A real PTY supplies the screen output; xterm.js renders it with a dark terminal palette for the PNG.
The application’s labels, state, mailbox rows and controls are unchanged.

Mermaid diagrams live next to their explanations in [visual patterns](../guide/patterns.md) and
[ready-made actors](../guide/actors.md). Their editable Markdown is the source; no generated diagram images are needed
for GitHub. Read the captions too: they identify commit boundaries and ordering limits.

## Refresh the TUI capture

Requirements: Python 3 on macOS/Linux, Node.js 22.12 or newer, and an installed Chrome/Chromium. Install rendering dependencies
outside the repository; this does not change the library's build or package dependencies.

```sh
cargo build -p cicada-sample-service -p cicada-app --features cicada-app/tui
export CICADA_DOCS_MODULES="$(mktemp -d)"
npm install --prefix "$CICADA_DOCS_MODULES" --ignore-scripts \
  @xterm/xterm@6.0.0 puppeteer@25.17.0 mermaid@11.17.2
export CICADA_DOCS_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
# On Linux, set CICADA_DOCS_CHROME to the installed Chrome/Chromium executable instead.
python3 scripts/capture-tui.py "$CICADA_DOCS_MODULES/tui.json"
node scripts/render-docs.mjs terminal "$CICADA_DOCS_MODULES/tui.json" docs/assets/tui.png
```

The capture starts a loopback-only node on an available port, runs two imports, pauses `sync/acme`, queues two more
messages and opens its actor screen. It cleans up its processes and store. The raw recording stays in the temporary
directory; commit only the checked PNG. A later library version will appear naturally when the capture is refreshed.

## Check Mermaid rendering

Using the same dependencies and browser:

```sh
node scripts/render-docs.mjs mermaid docs/guide/patterns.md "$CICADA_DOCS_MODULES/patterns"
node scripts/render-docs.mjs mermaid docs/guide/actors.md "$CICADA_DOCS_MODULES/actors"
```

The command parses every Mermaid block and writes temporary SVG/PNG previews. Inspect them for clipped labels,
misleading arrows and diagrams too wide to read. Compare the messages and commit boundaries with the linked source
and tests. Keep diagram syntax compatible with GitHub's Mermaid support.
