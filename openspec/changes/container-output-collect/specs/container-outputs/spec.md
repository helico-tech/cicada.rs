## MODIFIED Requirements

### Requirement: A store refuses names and paths outside the rules

Every `OutputStore` method SHALL fail with `Invalid` for a container name outside the container actor's name rule or
over 200 characters, and for a file path that is relative, has a `..` part, is `/` or holds a control character (NUL
included) in a part, before it touches anything, and SHALL treat paths that
differ only in empty and `.` parts as one file.

#### Scenario: A path from a user

- **WHEN** a host's route asks `read_file` for the path `/out/../../etc/passwd`
- **THEN** it fails with `Invalid` and reads nothing

#### Scenario: A NUL in a path

- **WHEN** a host asks `read_file` for a path with a NUL in it
- **THEN** it fails with `Invalid`
