## ADDED Requirements

### Requirement: Files can be copied out within limits

`copy_out(name, path, limits)` SHALL return the regular files at or below `path` of a created, running or ended
container, each whole with its absolute path and mode, or name it in `skipped` with a reason. A file larger than the
bytes left SHALL be skipped and the copy go on. An entry after the first `max_files` SHALL stop the copy and add one
skipped entry for the rest. A zero limit SHALL be `InvalidSpec`, a missing path an empty result, a missing container
`Conflict`.

#### Scenario: Collecting a folder

- **WHEN** an ended container wrote `/out/a` and `/out/sub/b`, and `copy_out("/out")` is called with room for both
- **THEN** it returns both files with their contents and modes, and nothing skipped

#### Scenario: One file too big

- **WHEN** a folder holds files of 10, 20 and 5000 bytes, and `copy_out` is called with `max_bytes` 100
- **THEN** it returns the two small files whole and names the big one in `skipped` with its size

#### Scenario: Too many files

- **WHEN** a folder holds five files, and `copy_out` is called with `max_files` 2
- **THEN** it returns two entries and one more skipped entry, for the folder, saying the rest was not read

#### Scenario: Exactly at the file limit

- **WHEN** a folder holds two files, and `copy_out` is called with `max_files` 2 and room for both
- **THEN** it returns both files and nothing skipped

#### Scenario: A zero limit

- **WHEN** `copy_out` is called with `max_files` 0 or `max_bytes` 0
- **THEN** it fails with `InvalidSpec` before it reads anything

#### Scenario: What is not there

- **WHEN** `copy_out` is called for a path that does not exist, for a missing container, and with a relative path
- **THEN** the first gives an empty result, the second fails with `Conflict`, and the third with `InvalidSpec`

### Requirement: A copy out reads only regular files

`copy_out` SHALL copy only regular files. It SHALL follow no symbolic link, inside the folder or out of it, and SHALL
skip with a reason every link, device, pipe or socket, and every entry whose name is not under `path`.

#### Scenario: Links in the folder

- **WHEN** a folder holds a file `a`, a link to `a` and a link to `/etc/passwd`
- **THEN** `copy_out` returns `a` and names both links in `skipped`

### Requirement: copy_out keeps other runtimes compiling

`copy_out` SHALL have a default implementation that fails with `Other`, so an implementation of `ContainerRuntime`
outside this repo compiles unchanged. The fake and the Docker backend SHALL implement it.

#### Scenario: A runtime written before copy_out

- **WHEN** a host's own runtime implements every method but `copy_out`, and `copy_out` is called on it
- **THEN** it compiles, and the call fails with `Other`

### Requirement: The fake holds files to copy out

`FakeRuntime` SHALL return from `copy_out` the files that `copy_in` wrote and those a test added with `put_file`, in
the order Docker's archive walks a folder, under the same limits and skip rules as the Docker backend. On an ended
container it SHALL leave out files written to a tmpfs.

#### Scenario: A file the program wrote

- **WHEN** a test adds `/out/report.txt` to `job-1` with `put_file` and calls `copy_out("job-1", "/out", …)`
- **THEN** it returns that file with its contents and mode

#### Scenario: A tmpfs after the end

- **WHEN** `job-1` has a tmpfs at `/scratch`, a test adds `/scratch/a` and ends the container, and calls
  `copy_out("job-1", "/scratch", …)`
- **THEN** it returns an empty result
