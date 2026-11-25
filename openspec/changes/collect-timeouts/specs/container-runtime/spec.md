## ADDED Requirements

### Requirement: A copy out reads at most a set number of bytes

`copy_out` SHALL stop reading once it has read `CopyLimits::max_read` content bytes of regular files, skipped ones
included,
keep the files it has, and add one skipped entry for the path that says the rest was not read. `CopyLimits::new` SHALL
set `max_read` to four times `max_bytes`.

#### Scenario: One huge file

- **WHEN** a folder holds a small file and then a file larger than `max_read`, and `copy_out` reads it
- **THEN** the small file is kept, the copy stops in the large one, and one skipped entry says the rest was not read
