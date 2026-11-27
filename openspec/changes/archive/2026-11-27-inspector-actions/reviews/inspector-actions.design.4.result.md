Verdict: ready for tasks

Both round-3 points are fixed in design.md.

- Section 6: the post-write soft refresh is queued as a new refresh behind a running one and never merged. Every pane read in it starts after the write's answer. The paging fallback is kept. A test covers it.
- Section 5: alert text is untrusted. A problem body gives title and detail. Any other body gives "HTTP <status>". Control characters and credentials are removed, and each part is cut at 500 characters. Tests cover all of this.

No new blocking gaps. Note: the section 5 sentence ending "and leaves the pane as it was" is grammatically broken. Fix it when the tasks are written.
Not run: the runtime and AppKit checks.
