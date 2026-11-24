## ADDED Requirements

### Requirement: Log pages come from Docker's timed stream

`logs_since` SHALL read Docker's log stream with timestamps from the whole second `S` of the cursor, drop lines
stamped before `S` whatever Docker's filter let through, skip the lines the cursor counts, and stop once the page is
full. Lines sharing a second, or out of time order within it, SHALL come back once each. A line Docker split at
16 KiB SHALL come back as several lines. A byte that is not UTF-8 SHALL become U+FFFD. A frame without a time SHALL
fail with `Other`.

#### Scenario: Lines across seconds while the program runs

- **WHEN** a container prints a numbered line every 300 ms for 3 seconds, and a host calls `logs_since` every
  500 ms with `max_lines` 2 and the last `next`, and once more after the container exited
- **THEN** the host has every numbered line once, in order

#### Scenario: A line stamped before the cursor's second

- **WHEN** the log holds A stamped in second T, then B stamped in T-1, then C stamped in T, a page ends after C, and
  Docker's `since=T` returns A, B and C
- **THEN** the next read drops B, skips A and C, and does not return C again

#### Scenario: Text that is not UTF-8

- **WHEN** a container prints the byte 0xFF and a newline
- **THEN** `logs_since` returns one line whose text is U+FFFD
