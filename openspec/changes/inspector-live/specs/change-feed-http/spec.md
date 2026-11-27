## ADDED Requirements

### Requirement: A client for the change stream

`ActorClient::changes` SHALL read `GET /changes?prefix=` as a stream of changed addresses and resyncs, with the client's
headers and token, SHALL end with an error when nothing arrives for 45 s, and SHALL NOT reconnect by itself.

#### Scenario: Hints and keep-alives

- **WHEN** the server sends a keep-alive comment, a `changed` event for `counter/a` and a `resync` event
- **THEN** the stream yields `Changed(counter/a)` and then `Resync`, and the comment yields nothing

#### Scenario: A silent server

- **WHEN** no byte arrives for 45 s
- **THEN** the stream ends with an error
