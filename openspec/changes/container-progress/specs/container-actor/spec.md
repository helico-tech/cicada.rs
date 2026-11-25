## ADDED Requirements

### Requirement: A container reports progress through a file

When `Start` names a `progressType`, the container actor SHALL set `CICADA_PROGRESS_FILE` to the job's progress path in
the container's environment, read that file at each poll while the container runs and once before its end is
delivered, and send `ContainerProgress { key, seq, text }` to the end's target as `progressType` whenever the text
changed, with `seq` counting from 1. A missing, oversized or unreadable file SHALL send nothing and SHALL not count as a
runtime error.

#### Scenario: Three updates

- **WHEN** a running container's program writes three different progress texts over several polls
- **THEN** the owner gets three progress messages with `seq` 1, 2 and 3, and none for a poll where the text did not
  change

#### Scenario: The last progress before the end

- **WHEN** the program writes progress and exits before the next poll
- **THEN** that progress message is sent in the turn that records the end, before the end message

#### Scenario: A NUL in the file

- **WHEN** the progress file holds a NUL byte
- **THEN** the message's text has U+FFFD in its place and is sent

#### Scenario: A progress path the runtime refuses

- **WHEN** `Start` names a `progressType` and the job's `PROGRESS_FILE` is a relative path
- **THEN** the container ends `failed` with the kind `notStarted` and nothing is made

#### Scenario: No progress asked for

- **WHEN** `Start` has no `progressType`
- **THEN** the actor sets no `CICADA_PROGRESS_FILE` and reads no file
