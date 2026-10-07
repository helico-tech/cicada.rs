# 0006. The console in the Sensus BPM Online design

Date: 2026-10-07

## Status

Accepted

## Context

ADR 0005 reused the React console of the Kotlin repo unchanged, in its Halo design. The Rust port's console should
look like the Sensus BPM Online design system (its tokens, fonts, components, line icons and rules such as "never colour
alone"), following approved mockups of the actors list, the actor page, the message page with its chain, the send
dialog and the failures page.

## Decision

The Rust console (`crates/cicada-ui/web`) is restyled to the Sensus BPM Online design system. Its behaviour, routes,
polling, keyboard shortcuts, theme switch, chain graph and list, schema forms and generated wire types stay the same.

- The system's `tokens.css`, `bundle.css` and fonts (Inter, Poppins, JetBrains Mono; open-font-licensed) are vendored
  unchanged in `web/src/styles/sensus/` and bundled by Vite as files; nothing loads from outside the origin.
- The console's components (`web/src/sx/`) emit the system's markup and `sx-` classes; the system's line icons are
  copied into its `Icon` component. Console-specific CSS (`web/src/styles/app.css`) covers layout and the chain graph
  and uses only the system's tokens.
- Light and dark mode set `data-theme` on the root element; density is compact.

The Kotlin console keeps its look.

## Consequences

- The web app of the Rust repo diverges visually from the Kotlin copy; the HTTP API, the fixtures test and the wire
  types are unchanged, so both consoles still read both servers.
- A behaviour fix in one console must be applied to the other by hand.
- The design system's tokens and fonts are vendored: an update of the system is copied in by hand. The bundle's
  `data:` URI masks are not used (the breadcrumb draws its chevron as an icon), so the strict Content Security Policy
  stays as it is.
