## ADDED Requirements

### Requirement: A collection that keeps timing out still ends

A collection turn whose step message has failed 3 or more times SHALL make no copy or log read, and SHALL record and
send the pending end with `outputs.error` naming the failures and the last error, and the counts of what is stored.

#### Scenario: A copy longer than the turn limit

- **WHEN** a container's output copy takes longer than the job's `MAX_TURN` on every try
- **THEN** after three timed-out tries the end is sent with the error in `outputs`, and no fourth copy is made
