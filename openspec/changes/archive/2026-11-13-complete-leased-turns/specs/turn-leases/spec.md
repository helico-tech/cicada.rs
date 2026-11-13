## ADDED Requirements

### Requirement: A trusted executor completes a leased turn with a report

ActorSystem::complete_turn SHALL accept actor, epoch, message id and a Returned, Failed or Cancelled report. It SHALL
derive the outcome and commit state and effects atomically through the existing fenced commit. Reports SHALL not
choose the input message's retry policy, arbitrary outcomes, sender identities or notification event ids.

#### Scenario: A returned handler

- **WHEN** a valid Returned report sets state and buffers sends, creates, cancellations, watches and controls
- **THEN** they commit together in the existing effect order, and Completed returns the applied outcome, state version,
  actual sent envelopes and cancelled ids
- **AND** no handler is executed by complete_turn

#### Scenario: No change, new state and stop

- **WHEN** a Returned report selects Unchanged, Set(value) or Stop
- **THEN** state/version remain unchanged, are updated, or the actor is removed according to existing turn semantics
- **AND** Stop returns the version before removal, while Set(null) is an invalid new value rather than Unchanged

#### Scenario: Derived send identity

- **WHEN** a report sends messages with caller-chosen ids
- **THEN** the server supplies the committing actor as from and the claimed message id as caused_by
- **AND** each send keeps its supplied id across an identical report retry

#### Scenario: Failed and cancelled reports

- **WHEN** a caller reports Failed or Cancelled
- **THEN** its report carries no user state or effects, and completion applies only the derived outcome and normal
  store-generated notifications

#### Scenario: Cancel wins before handler end

- **WHEN** the worker's existing handler-phase arbitration records cancellation before handler end
- **THEN** it may report Cancelled and the server commits cancelled without user effects
- **AND** a cancellation observed after handler end is ignored by the worker; the server does not arbitrate that race

### Requirement: Completion derives failure policy from the stored message

Only Handler, Node and Timeout failures SHALL be eligible for retry. Nonretryable reports MAY also use Refused,
Message, State and Commit. ClaimLimit or any other invalid combination SHALL fail before store access. For an eligible
failure the server SHALL use the stored message's retry policy and failure count, preserving the local executor policy.

#### Scenario: Failure of a currently running message

- **WHEN** a retryable report names a Running message of the actor and its policy permits another failure
- **THEN** completion reschedules the same message using failures+1 and the policy's capped backoff, with no user effects

#### Scenario: An expired but still held message

- **WHEN** status shows Pending because the lease expired, but this epoch is still held and current
- **THEN** completion may still use that message's policy and commit; a Running status is not required for validity

#### Scenario: No retry remains

- **WHEN** the report is nonretryable, the message has no policy, or its permitted attempts are exhausted
- **THEN** the derived outcome is failed, with the supplied cause and sanitized reason

#### Scenario: Bad failure combination

- **WHEN** the caller selects ClaimLimit or requests retry for Refused, Message, State or Commit
- **THEN** completion returns OpError::Invalid without store access or a lease mutation

#### Scenario: Status read fails or races replacement

- **WHEN** reading the retry snapshot fails
- **THEN** completion returns the store error without a commit
- **WHEN** the message changes through another commit or its lease is replaced after the snapshot
- **THEN** the old epoch cannot apply the derived retry; the commit fence remains authoritative

#### Scenario: Reason normalization

- **WHEN** a failure reason includes NUL or is longer than the reschedule error limit
- **THEN** NUL is replaced by U+FFFD and rescheduled lastError is truncated as in local execution
- **AND** terminal failure reasons retain the local policy without a new core length cap

### Requirement: Remote completion validates storage limits without changing handler checks

The server SHALL apply existing send-envelope, delivery, create-state and own-state storage checks to reports.
It SHALL not add JSON Schema or hosted-target decoding at completion. The executor SHALL retain call-time registry,
type, watch and buffering checks; a report SHALL describe its finished buffer rather than a sequence of Turn calls.

#### Scenario: An invalid send or creation value

- **WHEN** a send violates an existing envelope/delivery/value limit or a create supplies a null, NUL-containing or
  over-deep state
- **THEN** the server derives a no-effects nonretryable Commit failure using the existing refusal text
- **AND** that derived failure is still subject to the epoch and message-id checks at commit

#### Scenario: An invalid new own state

- **WHEN** Set contains null, NUL or a value beyond the state depth limit
- **THEN** completion derives a nonretryable Handler failure with the existing State cannot be stored prefix
- **AND** all buffered effects are discarded

#### Scenario: A server hosts a target implementation

- **WHEN** the same report is completed through servers with different hosted actor sets
- **THEN** those local sets do not change the completion's validation or result
- **AND** worker-side Serde, raw lookup and hosted-target checks remain that worker's responsibility

#### Scenario: Raw effects are not an operation log

- **WHEN** a manually constructed report includes a send and a cancellation of that send's id
- **THEN** the backend's existing cancel-before-send order applies
- **AND** the Rust executor's conversion instead preserves Turn's existing rule that cancelling a buffered send
  removes it before a report is produced

#### Scenario: Identity and structural refusal

- **WHEN** a report names the wrong claimed message, cancels its own input, conflicts on child/control effects or
  violates a target/id/ancestor rule
- **THEN** the existing backend checks refuse atomic application
- **AND** no promise is made that epoch fencing precedes every structural request check

### Requirement: A completion call makes at most one commit attempt

Completion SHALL return Completed on confirmed commit and LeaseLost on the store's lost-lease result. A known
rejection SHALL return Rejected with the exact nonretryable failure report to submit next. An uncertain store error
SHALL not cause a different commit, release or internal retry. The caller SHALL retain its current report across retries.

#### Scenario: A known rejection

- **WHEN** the store rejects effects or refuses their data and no commit applied
- **THEN** completion returns Rejected with the shared executor fallback cause and reason, without committing it
- **AND** the caller replaces its current report with Failed(that failure), permanently discarding the original effects

#### Scenario: A refused reschedule

- **WHEN** a derived retry's delivery is outside the store's time range
- **THEN** Rejected carries the original handler cause and reason with the refusal appended, marked nonretryable
- **AND** submitting that failed report can end the message without effects

#### Scenario: A refused terminal failure

- **WHEN** a no-effects terminal failed decision is itself refused, including one with a wrong message id
- **THEN** completion returns OpError::Invalid rather than another fallback report
- **AND** no outcome or effects are applied and the caller must abandon without release

#### Scenario: Failure before the primary commit applies

- **WHEN** a store error prevents the primary commit from applying
- **THEN** completion returns the error without release, fallback or handler execution
- **AND** the caller may retry its identical report through any server within its conservative deadline

#### Scenario: Failure after the primary commit applies

- **WHEN** the primary commit applies but its acknowledgement is lost
- **THEN** a retry of the same report cannot duplicate state, sends, outcomes or watcher notifications
- **AND** the store's LeaseLost response is returned without claiming it proves the first attempt succeeded

#### Scenario: Cancellation of the completion request

- **WHEN** the caller drops complete_turn while its backend commit is in flight and retries the identical current
  report through another system on the same store
- **THEN** no release occurs and at most one outcome and effect set apply
- **AND** the caller treats the dropped request as uncertain, including a SQLite commit that settles after cancellation

#### Scenario: A lost rejection response

- **WHEN** a rejection response is lost before the caller learns of it
- **THEN** no failure fallback was attempted by the server
- **AND** retrying the original report may re-evaluate its effects against the current store, including a target
  created since the unacknowledged rejection

#### Scenario: An acknowledged rejection followed by an uncertain failed commit

- **WHEN** the caller receives Rejected, switches to the provided Failed report, and that failed commit's response is lost
- **THEN** every later retry uses only that Failed report, even if the original effects would now be valid
- **AND** the original state/effects can never reappear through the compliant caller's retry path

#### Scenario: Concurrent reports

- **WHEN** identical or differing reports race for one actor epoch
- **THEN** at most one commit applies and clears the lease, and later attempts cannot apply their effects

### Requirement: Completion preserves lifecycle and acknowledgement uncertainty

complete_turn SHALL work without server initialization, through any peer and after node stop while the store is
available. It SHALL never release the lease or notify server observers. Only an acknowledged Completed SHALL let the
worker finish its observers. The caller SHALL not rerun a finished handler or release it after any uncertain completion.

#### Scenario: Peer and stopped servers

- **WHEN** a lease from one system is completed through an unstarted or stopped peer with different configuration
- **THEN** the same report policy and atomic checks apply without process-local lease state

#### Scenario: Retention hides the result

- **WHEN** a completion applied but retention hides its outcome before a lost reply is retried
- **THEN** the epoch still prevents duplicate effects
- **AND** a message-status read is not promised to resolve the caller's uncertainty

#### Scenario: Server observers and wakes

- **WHEN** completion commits or may have committed without acknowledgement
- **THEN** it wakes local dispatch as appropriate but does not start/end server observers for that remote turn
- **AND** the worker ends its own observer only on Completed, otherwise eventually dropping it on abandonment or lease loss

#### Scenario: Closed store

- **WHEN** completion reaches a closed store
- **THEN** it returns the ordinary unavailable error and performs no release or automatic fallback
