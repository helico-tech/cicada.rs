## ADDED Requirements

### Requirement: Live updates

The inspector SHALL follow the change stream for the page on screen, reload that page at most about twice a second
when hints arrive and every 30 s while live, read the pane again when a hint names its actor, keep the selection, the
scroll position and the open rows across these reloads, and show Live, Reconnecting or Paused, which the user can
pause and resume.

#### Scenario: A hint for the selected actor

- **WHEN** the Actors page shows actor `counter/a` selected and the server sends a `changed` hint for it
- **THEN** within a second the pane shows its new state and the selection and scroll position are unchanged

#### Scenario: A burst of hints

- **WHEN** fifty hints arrive within 200 ms
- **THEN** the page reloads once

#### Scenario: Paused

- **WHEN** the user pauses and hints arrive
- **THEN** nothing reloads until the user resumes, and then the page reloads once
