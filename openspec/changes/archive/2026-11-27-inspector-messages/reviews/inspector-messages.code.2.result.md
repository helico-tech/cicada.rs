Verdict: pass

Each Messages history entry restores its own prefix and selection, including after a prefix reload. Switching contexts clears both directions. The envelope scrolls above Show Actor and the payload. An exhausted search clears the pane when the selected message is absent. Resize, search, and menu validation release their borrows before calling AppKit. The formatting test holds a running blocking task, displays the next selection, then verifies that the late result leaves the pane unchanged.

All 49 inspector tests and one doctest passed. The diff passed whitespace checks. Visual AppKit checks, the workspace suite, and the gate were not run.
