## ADDED Requirements

### Requirement: A client for the change stream

`ActorClient::changes` SHALL read `GET /changes?prefix=` through the client's own HTTP client as a stream of changed
addresses and resyncs, SHALL bound the handshake to 10 s, SHALL end with an error when no byte arrives for 45 s, and
SHALL NOT reconnect by itself.

#### Scenario: Hints and keep-alives

- **WHEN** the server sends a keep-alive comment, a `changed` event for `counter/a` and a `resync` event
- **THEN** the stream yields `Changed(counter/a)` and then `Resync`, and the comment yields nothing

#### Scenario: A silent server

- **WHEN** no byte arrives for 45 s
- **THEN** the stream ends with an idle error

#### Scenario: A client with a short total timeout

- **WHEN** the client was built with a total timeout of 30 s and the server keeps sending keep-alives for 2 minutes
- **THEN** the stream stays open
