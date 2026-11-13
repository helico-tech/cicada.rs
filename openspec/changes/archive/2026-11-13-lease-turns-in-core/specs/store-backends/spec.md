## ADDED Requirements

### Requirement: Renewing a held lease never shortens its expiry

For a held current epoch, every backend's renew SHALL atomically set expiry to the later of its existing expiry and
store now plus the requested TTL, and return that actual expiry. Both ordinary and cancel-requested renewals SHALL
obey this rule. Existing epoch/held-lease fencing and store-time precision SHALL remain unchanged.

#### Scenario: A shorter extension request

- **WHEN** a lease expires at t=60 and renewal at t=10 requests 5 seconds
- **THEN** its expiry remains t=60 and the renewal result returns t=60
- **AND** another claim before t=60 is excluded even if the renewal answer never reaches the holder

#### Scenario: A longer extension request

- **WHEN** a lease expires at t=60 and renewal at t=10 requests 80 seconds
- **THEN** its expiry becomes t=90 and the renewal result returns t=90

#### Scenario: Concurrent shorter and longer renewals

- **WHEN** renewals for one held epoch race with different requested intervals
- **THEN** each serializes through the backend's atomic write and no successful renewal reduces the stored expiry

#### Scenario: A cancellation request survives renewal

- **WHEN** cancellation was requested for the held epoch and a shorter renewal is applied
- **THEN** the result still reports cancellation and the retained maximum expiry

#### Scenario: Fencing and expired current leases

- **WHEN** an epoch is stale or no longer held
- **THEN** renewal still returns LeaseLost without changing a newer lease
- **WHEN** an expired epoch remains held and current
- **THEN** renewal may extend it under the same maximum rule
