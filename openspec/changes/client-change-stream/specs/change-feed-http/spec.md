## ADDED Requirements

### Requirement: A client for the change stream

`ActorClient::changes` SHALL read `GET /changes?prefix=` through the client's own HTTP client as a stream of changed
addresses and resyncs. Opening it SHALL fail with the API's problem for a status other than 200, with a body error for
another content type, and with a timeout after 10 s without response headers. The stream SHALL end with an error when
no byte arrives for 45 s or its data is malformed, SHALL end without one when the body ends, and SHALL NOT reconnect by
itself.

#### Scenario: Hints and keep-alives

- **WHEN** the server sends a keep-alive comment, a `changed` event for `counter/a` and a `resync` event
- **THEN** the stream yields `Changed(counter/a)` and then `Resync`, and the comment yields nothing

#### Scenario: A silent server

- **WHEN** no byte arrives for 45 s
- **THEN** the stream ends with an idle error

#### Scenario: A client with a short total timeout

- **WHEN** the client was built with a total timeout shorter than the stream's life and the server keeps sending bytes
- **THEN** the stream stays open

#### Scenario: Refused

- **WHEN** the server answers 401 to the request
- **THEN** `changes` returns the API error with status 401, and no stream

#### Scenario: Not an event stream

- **WHEN** the server answers 200 with an HTML page
- **THEN** `changes` returns a body error

#### Scenario: Malformed data

- **WHEN** the server sends a `changed` event whose data has no address
- **THEN** the stream ends with a malformed error

#### Scenario: The store closes

- **WHEN** the server ends the body after a `resync` event
- **THEN** the stream yields `Resync` and then ends without an error

#### Scenario: No answer

- **WHEN** the server accepts the connection but sends no response headers for 10 s
- **THEN** `changes` returns a body error saying no response headers came within 10 s
