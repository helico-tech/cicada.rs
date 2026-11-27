Verdict: fix needed

1. model.rs:393,492 and live.rs:442,548. Switch contexts on Overview as a batch from the old stream comes due. The visit stays the same, so the old driver can queue a soft refresh behind the new context's initial hard refresh before processing Source. Source leaves that request in place. The new server gets an extra reload even if its stream never sends resync. Fence refresh starts by context as well as page visit.

2. actors.rs:425,673,1384. Press ⌘R on a selected actor as it disappears. When its lookup returns 404, the pane marks every part complete while the other requests keep running. The timer can start early. A scratch test reproduced this with a held mailbox response. Wait for those reads to end or cancel them.

Notes. The Types test matches the design: the old pane stays intact until fenced publication. Pause and Resume serialize correctly on the AppKit thread. Back and Forward fence section changes, close stops the driver before cancelling reads, and a page switch drops the prior reload wait. At outcomes.rs:439,463, a newer Overview count ticket can leave a finished Outcomes count in its task slot. The AppKit page switch drops the affected wait.

Not checked. Full suite, gate, E2E, AppKit rendering, scroll retention and VoiceOver.
