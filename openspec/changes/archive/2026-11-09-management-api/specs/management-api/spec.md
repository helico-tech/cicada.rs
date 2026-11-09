## ADDED Requirements

### Requirement: Two routers separate reading from changing

The library SHALL offer a router with every route and a router with only the routes that read. Both SHALL be axum
routers that the host mounts and wraps in its own authentication.

#### Scenario: A write on the read router

- **WHEN** a client sends `PUT`, `POST` or `DELETE` to a route of the read router
- **THEN** the answer is `405` and nothing changes

### Requirement: The API publishes an OpenAPI document

Each router SHALL serve an OpenAPI 3.1 document of its own routes at `GET /openapi.json`. A copy of the full document
SHALL be committed, and a test SHALL fail when it differs from the routes.

#### Scenario: The document describes every error

- **WHEN** a client reads an operation in the document
- **THEN** it has an operation id and describes errors as `application/problem+json`

#### Scenario: A route changes

- **WHEN** a route or wire type changes and the committed copy does not
- **THEN** the test fails
