Verdict: ready for tasks

The standalone lifecycle follows the landed tray’s runtime and timer pattern. Embedding has its own proposal. ADR 0076 preserves ADR 0056 and permits the future optional tray edge. `resolve` and `Target::client` provide the selected context, credentials, and timeout. The existing client supports the stats prefixes and turns preview. This fits five tasks: context client, portable model, AppKit shell, Overview rendering, and ADR/gate/docs handoff.

Task notes

- Design §§3–4. Show active environment overrides as tray Settings plans to do. Keep the window usable after a target error and keep the picker aligned with the displayed data.
- Design §3. Use safe error summaries for transport and malformed URLs. `ActorClient::url` embeds the raw base in parse errors, and the usage redactor misses some malformed credentials. Test both paths.
- Design §2 and bounded refresh spec. Cancel superseded refreshes, preserve completed results at the deadline, and mark overdue base routes timed out. Specify that “not counted” covers the cap or deadline. Explicit count request errors remain failures.
- Design §5. Verify Close and Quit both clean up on macOS.

Strict OpenSpec validation passed. Builds, tests, mockup comparison, and live AppKit checks were not run.
