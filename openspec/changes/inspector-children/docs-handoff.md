# Docs handoff for inspector-children

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The Actors page shows an outline. Opening a row reads the direct children created by that actor. Each open row pages
its children independently. Selecting a child fills the existing inspector pane.

The tests in `crates/cicada-inspector/src/tests/actors/children.rs` cover child paging, retries, stale reads, separate
selection and folding for each occurrence, the running sample, and restoration across root and child pages. The
native window check list is in `tasks.md`.

## 1. Folding rows in the Actors page

**Where.** Add to the Actors description in `docs/guide/app.md`, under `Inspector on macOS`.

**Text.** Open an Actors row with its disclosure triangle or the right arrow key to see its first 100 children.
Choose **more…** for the next page of that parent's children. A failed read offers **Retry** under the parent.
Closing a row keeps its loaded pages so you can reopen it without another read. The root list still loads more rows
as you scroll near the end of its loaded pages.

A child row shows its address, type, and running status. A dash (—) means its address is outside the running
sample's prefix. Select the row to see the rest of the actor's details in the pane. An actor can appear in the root
list and under a parent. Each row folds and selects independently.

⌘R reloads the root list and reopens surviving rows as their pages load. A different search prefix or context starts
with all rows folded. If you type and erase text before the search settles, the original prefix keeps its folds.

**Caveats.** Scroll to a later root page or choose **more…** under a parent to reach a previously open row on that page.
A row that has disappeared is forgotten once a page passes its address or reaches the end of its list.

## Blocking

None.

## Status

Proposed app guide text. The inspector built and passed Clippy and all 19 Actors model tests. All 11 documentation
tests and 54 doctests passed. Workspace formatting, Clippy, 3,905 tests with PostgreSQL, and strict OpenSpec
validation passed. The change loop runs the gate at landing.

An AppKit smoke run exercised parent expansion and more…, arrow-key folding, independent fold and selection in
duplicate rows, the child pane, fold restoration through the Refresh action, prefix reset, and root paging on
scroll. The ⌘R shortcut dispatch, broader keyboard and VoiceOver checks, and visual checks in light and dark
still need a manual pass.
