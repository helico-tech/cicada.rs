# Proposal

## Why

Many outside systems can only be asked, not listened to: an API behind a firewall, a status page, a feed, a tracker
whose webhooks the host may not configure. A host that wants actors to react to a change there writes the polling,
the "what did I see last" bookkeeping and the recovery after a crash by hand. An actor is a good fit: its durable
state is the marker of what it saw last, so a restart misses nothing and repeats nothing inside the system.

This is the second of three generic integration actors, after `webhook-sink`. Later monitors for one vendor (a Jira
project, the issues of a GitHub repository) are this actor with a vendor-specific request and cursor.

## What Changes

- **An HTTP poller actor** (`cicada.http-poller`), one instance per watched URL. Its state holds the URL, fixed
  headers, the name of a secret, a target with a stored message, and the marker of the last response.
- **A public `Poll` message** makes one `GET`. A scheduler instance sends it on its slots; the poller keeps no timer
  of its own. A host can also send `Poll` by hand.
- **Change detection.** The poller sends the stored `ETag` as `If-None-Match` when it has one. A `304`, or a body with
  the same hash as the last one, commits with no send. A changed response saves the new marker and sends the target
  its stored message with the new body, in the same commit. So a target sees each detected change once.
- **Subscribing.** The target is one address. For several interested actors the host names a topic as the target and
  they subscribe to it. The poller keeps no subscriber list.
- **Failures.** A failed call fails the turn and follows the retry policy of the `Poll` message. The next slot polls
  again in any case, so the design can keep retries short.
- It reuses the secret resolver and the HTTP client of `webhook-sink`, and lives in the same crate.

## Out of scope

Vendor cursors, pagination and "list what changed since" queries: they belong to the vendor monitors. Diffing two
bodies or reporting which field changed. Keeping a history of responses. Bodies larger than a limit the design sets:
state is rewritten each turn, so the poller stores a hash, not the body. Methods other than `GET`.

## Order

After `webhook-sink`. Before `webhook-inbox`.

## Impact

A new actor with its contract, tests on virtual time against a fake endpoint, a worked example with a scheduler and
a topic, and a handoff to the docs session for the actors guide. No change to the core, the store, a guarantee or an
existing wire format.
