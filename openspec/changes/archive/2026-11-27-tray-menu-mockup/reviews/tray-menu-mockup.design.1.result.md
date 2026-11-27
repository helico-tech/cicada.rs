Verdict: not yet

Blockers

1. Decision 3 omits offline selection. Disconnected updates the highlight and ⌘K target without connecting. MenuControl connects immediately, which the existing spec requires after failure. Specify selection and amend the spec. Keep context identity separate from the title when joining subtitles on older macOS.
2. Decision 5 must keep Stop Now actionable for a ready worker, with separate disabled help and ⌘Q below. The current deadline includes cancellation time. Define the bar for zero grace and early cancellation, and preserve listings for stranded turns after going offline.
3. Decision 4 omits the failed row’s message type and retry time. Define rescheduled, cancelled and abandoned rows and the failure count. List any unavailable retry time, disconnecting icon dimming and feature-gated Inspector as departures.
4. Decision 3 needs a failure timestamp and safe display text. Worker HTTP errors can contain request URLs. Test that display excludes credentials and queries.
5. Decision 1 can combine plain titles, images, string badges (14.0) and subtitles (14.4). Use non-template circles per appearance and request image visibility on macOS 27, which typically hides them. List native sizing and highlight departures.

Notes

1. Both APIs fit. Store only parsed host and port in Contexts. Take concurrency from the started worker’s config.
2. Five tasks fit across app APIs, tray state, portable model, AppKit, and verification with handoff.
3. OpenSpec validated. Menu sizing checked on macOS 27. Sonoma rendering untested.
