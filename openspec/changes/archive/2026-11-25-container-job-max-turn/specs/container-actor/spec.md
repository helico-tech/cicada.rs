## ADDED Requirements

### Requirement: A container job sets the turn limit of its actor

`ContainerJob` SHALL have `const MAX_TURN: Option<Duration>` with the default `None`, and `ContainerActor<J>` SHALL use
it as its `Actor::MAX_TURN`, so it wins over the node's default turn limit; `None` SHALL keep the node's default.

#### Scenario: A slow pull fits

- **WHEN** a job sets `MAX_TURN` to 35 minutes, the node's default is 15 minutes, and the create step of a container
  takes 20 minutes
- **THEN** the step ends `done` and the container starts

#### Scenario: No limit set

- **WHEN** a job does not set `MAX_TURN`
- **THEN** its actor uses the node's default turn limit

#### Scenario: A limit below the create step

- **WHEN** a job sets `MAX_TURN` to 1 minute and the create step takes 2 minutes
- **THEN** the step ends `failed` with the cause `timeout`, its retry runs the step again, and no second container is
  made

#### Scenario: A zero limit

- **WHEN** a host registers a container job whose `MAX_TURN` is zero
- **THEN** the registration is refused with "MAX_TURN must be positive"
