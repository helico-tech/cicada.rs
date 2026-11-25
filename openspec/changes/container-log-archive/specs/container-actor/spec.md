## ADDED Requirements

### Requirement: The actor labels its containers with its prefix

`ContainerActor` SHALL add the label `cicada.container.prefix` with the settings' prefix to the spec of every container
it creates, after the job's `spec`, so a job cannot remove it.

#### Scenario: A job with its own labels

- **WHEN** a job's spec sets the label `team=ci` and the actor creates the container
- **THEN** the container carries `team=ci` and `cicada.container.prefix` with the prefix
