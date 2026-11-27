Verdict: not yet

- Decision 1 in design.md still assigns rescheduled turns an orange icon. Decision 4 matches Running’s red cross and “<error>, retry HH:MM”, and counts the retry in Main’s “N failed”. Make decision 1 use the same red cross.
- The offline pick fits MenuControl. AppKit reloads the context list in menuNeedsUpdate as the menu opens. The pick starts from the config’s current context, survives reload while its name exists, and stays in memory.
- The new tray data is crate-private. Before readiness, both the bar and Stop Now are hidden. Delayed retries use finish time plus delay. An empty context list shows “Run cicada config” and hides Connect.
- Five tasks fit across app APIs, tray state, menu model, AppKit, and verification with docs handoff.

Strict OpenSpec validation passed. AppKit rendering remains unchecked.
