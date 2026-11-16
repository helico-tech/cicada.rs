# Design review resolution

Review: `actor-design-guide.design.1`, 2026-11-16. Original verdict: not yet; one blocker and one note.

- **NUL submission boundary, fixed.** Confirmed ActorSystem::send_message_checked calls check_json before enqueue,
  and Turn::buffer calls check_send before push_send. The design, spec and proposed ADR now distinguish
  OpError::Invalid/TurnError::Invalid with no queued Enqueue or session reply from actor-level InvalidBody for an
  oversized, otherwise storable string. The test plan and docs handoff explicitly carry that distinction.
- **Root README navigation, fixed.** Added root README to the docs owner's navigation handoff in the design, spec
  and ADR, preserving the existing documentation specification's overview requirement.

These are narrow corrections to the reviewed scope and protocol, not a substantial redesign; no second review is
requested. The original report remains unchanged. The owner approved the corrections, tasks and Ready commit on
2026-11-16, and delegated subsequent design decisions to this session with Opus 5.5 for new reviews.
