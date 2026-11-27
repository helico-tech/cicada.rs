## ADDED Requirements

### Requirement: Live updates

The inspector SHALL follow one change stream for the page on screen, run that page's soft refresh at most about twice a
second from batched hints and 30 s after its last refresh attempt, reconnect with backoff, and show Live, Reconnecting
or Paused; pausing SHALL stop the stream, the timer and any automatic refresh in flight until the user resumes, while a
manual Refresh (⌘R) keeps working.

#### Scenario: A hint for the selected actor

- **WHEN** the idle Actors page shows actor `counter/a` selected and the server sends a `changed` hint for it
- **THEN** a soft refresh starts within a second, and when it ends the pane shows the new state with the selection and
  scroll position unchanged

#### Scenario: A burst of hints

- **WHEN** fifty hints arrive within 200 ms
- **THEN** the page runs one soft refresh

#### Scenario: Paused

- **WHEN** the user pauses and hints arrive
- **THEN** nothing reloads by itself until the user resumes (⌘R still does), and then the stream's first `resync` runs
  one soft refresh

#### Scenario: The stream drops

- **WHEN** the change stream ends
- **THEN** the pill shows Reconnecting, the page still refreshes every 30 s, and a new stream opens after the backoff
