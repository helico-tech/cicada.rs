## ADDED Requirements

### Requirement: Stop Now in the menu

While the tray stops a worker that was ready, the menu SHALL offer Stop Now, and `Tray::stop_now` SHALL end the
worker's grace period: its running turns SHALL be cancelled and listed as abandoned, and the state SHALL become
`Offline` once the worker returns. In any other state `stop_now` SHALL do nothing and the menu SHALL not offer it.

#### Scenario: Stop Now during the grace period

- **WHEN** a turn runs, the tray disconnects with a grace period of 30 seconds, and the user picks Stop Now
- **THEN** the turn is listed as abandoned and the tray is `Offline` within the cancel timeout

#### Scenario: Stop Now while online

- **WHEN** the tray is online and `stop_now` is called
- **THEN** nothing changes and the menu offers no Stop Now
