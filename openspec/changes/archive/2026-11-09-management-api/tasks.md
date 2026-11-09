# Tasks

- [x] 1 Wire types derive `JsonSchema`. Verify: `cargo build -p cicada`.
- [x] 2 Routes described with aide; `actor_api_read`; `GET /openapi.json`; `GET /types/{name}`. Verify: `crates/cicada-axum/tests/openapi.rs`.
- [x] 3 Committed `docs/openapi.json` and its drift test. Verify: `the_committed_document_is_current`.
- [x] 4 Client: `get_type`, and every route it calls is documented. Verify: `the_client_only_calls_documented_routes`.
- [x] 5 README, architecture, changelog. Verify: doctests of `cicada-examples`.
