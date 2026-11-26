Verdict: fix needed

## Blocking

1. `crates/cicada-actors/src/placement.rs:503-509` (`check`): a Check that runs before `armed` arms nothing. Once that happens, `armed` stays `Some` and no Check is pending. `offer` arms only when `armed` is `None` (line 477), so offers never restart the chain. A node that goes quiet is then never marked silent and no notice goes out. The state looks normal, so nobody knows to send `Rearm`. This breaks the requirement "A node that goes quiet is silent, once."
   Without an operator it cannot happen: the claim reads store time after taking its locks and claims only due messages, and store time never goes back (S9). `turn.now()` is that claim time. But `deliver-now` on the pending Check does trigger it, and it is the obvious way to "check now" (CLI `deliver-now`, the terminal UI, `POST /messages/{id}/deliver-now`, `ActorSystem::deliver_now`).
   To see it: offer `mac-1`, run `test.system().deliver_now(check_id)` at +10 s, then advance 600 s. `mac-1` stays `Live` with no notices, and `armed` is `00:01:00` with no Check pending. I ran this probe and it fails as described.
   The design says Rearm works "as in the heartbeat watch". The heartbeat watch covers this case: it keeps the armed timer's `MessageId`. That timer re-arms itself when it runs early, every other timer message is ignored, and `Rearm` cancels the old one before it arms a new one. Fixed means doing the same here: keep the armed Check's id (and its time), make the armed Check re-arm when it runs early, have strays change nothing, and have `Rearm` cancel and replace. That also removes the two-checks-after-Rearm case. Add a test of a Check made due early, and update the contract and samples.

## Notes

1. The rustdoc at lines 410-412 says an early check "marks what is due". In fact every live node's deadline is at or after `armed`, so an early check marks nothing.
2. Each `Rearm` sent while a check is pending adds one more no-op Check at the same time. These are harmless and go away with fix 1.
3. Tasks 1-5, the four spec scenarios (each has a test), the versions (0.24.0 everywhere), the CHANGELOG, the generated references, the doc comments and the exports are fine. No protected docs are touched. The handoff is accurate apart from its "A lost check" point, which needs the fix 1 behaviour.

## Not checked

I did not run the suite, the gate or PostgreSQL. The only test I ran was the probe above. I did not compare the contract JSON with the code field by field.
