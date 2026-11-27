## ADDED Requirements

### Requirement: Stop Now in the menu

While the tray stops a worker that was ready, the menu SHALL offer Stop Now, and `Tray::stop_now` SHALL end the
worker's grace period: its running turns SHALL be cancelled, and the state SHALL become `Offline` once the worker
returns, within the cancel timeout. In any other state `stop_now` SHALL do nothing and the menu SHALL not offer it.

#### Scenario: Stop Now during the grace period

- **WHEN** a turn runs, the tray disconnects with a grace period of 30 seconds, and the user picks Stop Now
- **THEN** the turn is cancelled and the tray is `Offline` within the cancel timeout

#### Scenario: Stop Now while online

- **WHEN** the tray is online and `stop_now` is called
- **THEN** nothing changes and the menu offers no Stop Now

### Requirement: Turns after Stop Now

After Stop Now, a cancelled turn whose handler had not ended SHALL be listed as abandoned once its lease is given back,
and a turn whose handler had ended MAY still commit. A turn the worker leaves behind SHALL stay listed as running until
it ends; `Offline` SHALL NOT wait for it.

#### Scenario: A cancelled turn gives its lease back

- **WHEN** the user picks Stop Now while a turn's handler runs
- **THEN** the turn is listed as abandoned once its lease is given back, and its message is pending again

#### Scenario: Stop Now while the release is slow

- **WHEN** the user picks Stop Now and the cancelled turn's release is still in flight at the cancel timeout
- **THEN** the tray is `Offline` at the cancel timeout, the turn stays listed as running, and it is listed as
  abandoned once the release ends
