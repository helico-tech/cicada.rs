# Docs handoff for placement-settings

Version: 0.25.0 (unreleased batch). Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

A placement pool's silence, node limit, line limit and holder limit can change on a live pool. The new message
`PlacementMsg::Configure { silence, max_nodes, max_waiting, max_holders }` (crate `cicada-actors`), each field an
`Option`, from anyone, replaces the settings it names within the bounds of `PlacementState::new` and keeps the others.
On the wire: `{"type": "Configure", "silence": "PT2M", "maxHolders": 8}`; `silence` is an ISO-8601 duration and the
limits are `maxNodes`, `maxWaiting`, `maxHolders`. There is no helper: Rust code builds the variant.

- A message with no field set, a value out of bounds (silence 1 second through 1 day, `maxNodes` and `maxHolders` 1
  through 1,000, `maxWaiting` 0 through 1,000), or a silence after which a node's or contest's silence would end past
  the last representable instant, is refused for good and changes nothing. A misspelled field reads as absent: alone it
  makes an empty message (refused); next to a valid field it is ignored, as unknown fields are in every pool message.
- A lower limit keeps what is there: the known nodes, holders, waiting asks and contests stay, and new ones are refused
  or queued as at the limit until the count is below the new limit. The stored state may now hold more than its
  limits, up to 1,000 each.
- A larger `maxHolders` grants waiting asks in the same turn. A larger `maxNodes` admits a new node at its next offer;
  offers refused before stay refused.
- A silence different from the current one, with a check armed, arms the check at the new earliest end of a live
  node's silence or of a contest, cancelling the armed one; a time that has passed is due at once, so a shorter
  silence marks the quiet nodes silent in the next turn, not in the `Configure`'s own. An equal silence arms nothing.
  Conflict marks and contests end one new silence time after their last offer or report.

Proved by `crates/cicada-actors/tests/placement.rs` (`a_configure_replaces_the_settings_it_names_and_keeps_the_others`,
`a_configure_without_a_setting_or_out_of_bounds_is_refused_and_changes_nothing`,
`a_lower_holder_limit_keeps_the_holders_and_queues_new_asks_until_under_it`,
`a_higher_holder_limit_grants_the_waiting_asks_in_the_same_turn`,
`a_lower_node_or_line_limit_keeps_what_is_there_and_refuses_new_ones`,
`a_shorter_silence_marks_a_quiet_node_silent_in_the_next_turn`,
`a_longer_silence_moves_the_armed_check_later_and_an_equal_one_arms_nothing`,
`the_documented_cicada_send_payload_configures_the_pool`), the unit tests in `crates/cicada-actors/src/placement.rs`
and `crates/cicada-actors/tests/wire_formats.rs`. The contract reference `docs/reference/contracts/cicada.placement.json`
is regenerated.

## 1. The settings are no longer fixed

**Where.** `docs/guide/actors.md`, `Placement pool`, step 1 of the list ("The settings are fixed at the create; creating
a pool that exists changes nothing.").
**Text.** Replace the fixed-settings sentence with: the settings are given at the create, and `Configure` changes them
on a live pool (see the new paragraph below); creating a pool that exists still changes nothing.
**Caveats.** None.

## 2. A new paragraph: changing the settings

**Where.** `docs/guide/actors.md`, `Placement pool`, a new bold-titled paragraph such as `**Changing the settings.**`,
near `**Fewer places.**` (the keep-what-is-there rule is the same idea).
**Text.** Send `PlacementMsg::Configure { silence, max_nodes, max_waiting, max_holders }` with the fields to change as
`Some` and the rest `None`; it takes the create's bounds and comes from anyone. A message with no field, or a value out
of bounds, is refused for good and changes nothing. A lower limit removes nothing: the nodes, holders and waiting asks
stay, and new ones are refused (a new node, a new contest, an ask that must wait while the line is full) or queued (an
ask while the pool holds `maxHolders` or more) until the count is below the new limit. A higher `maxHolders` grants
waiting asks at once; a higher `maxNodes` admits a new node at its next offer. A new silence re-arms the check at the
new earliest end: a shorter one marks the quiet nodes silent in the next turn (a `Configure` that also raises
`maxHolders` may grant a place on a node that the next turn marks silent), a longer one moves the check later.
**Caveats.** Keep the silence above the members' offer interval (`MemberSettings`, 15 s by default), or nodes flip
between silent and back. Unknown fields are ignored, so a misspelled field next to a valid one changes nothing; read
the state afterwards to confirm.

## 3. The `cicada send` recipe

**Where.** `docs/guide/actors.md`, `Placement pool`, `**From the command line.**`: the opening sentence ("An operator
drains, undrains, declares gone and releases") and the `sh` block.
**Text.** Add configuring to the sentence and a line to the block, as proved by
`the_documented_cicada_send_payload_configures_the_pool`:

```sh
cicada send cicada.placement/runners --type Configure --payload '{"maxHolders": 8, "silence": "PT2M"}'
```

In the bullet list below it, the refused-later bullet can name an empty `Configure` (or one with only a misspelled
field) next to the `Gone` and `Drain` examples.
**Caveats.** `silence` is an ISO-8601 duration string (`"PT2M"`), not a number of seconds; a number is refused at once
with `422`.

## 4. Silence paragraph

**Where.** `docs/guide/actors.md`, `Placement pool`, `**Silence.**`.
**Text.** One sentence: a `Configure` with another silence re-arms the check at the new earliest end, at once when that
has passed.
**Caveats.** None.

## 5. Upgrade guide

**Where.** `docs/how-to/upgrading.md`, `Upgrade to 0.25.0`, the Rust paragraph ("and a `Duplicate` arm to exhaustive
matches of `PlacementMsg`").
**Text.** Make it "a `Duplicate` and a `Configure` arm to exhaustive matches of `PlacementMsg`".
**Caveats.** No other upgrade step: the contract already changes in 0.25.0 and its maintenance window covers this.

## 6. Changelog

**Where.** `CHANGELOG.md`, `0.25.0 — unreleased`.
**Text.** Done by the builder: the summary names `Configure`, `### Added` has its entry, and `### Changed (breaking)`
names the new `PlacementMsg` variant.
**Caveats.** None.

## Blocking

None.

## Status

`cargo test --workspace` (with PostgreSQL) passes, as do `cargo test -p cicada-examples --test docs` and `--doc` with
the guides unchanged. Items 1–5 are proposed text for the docs session; no protected file was edited.
Item 6 is in this branch.
