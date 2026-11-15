//! [`ActorClient`] as a [`TurnSource`] (ADR 0040): what a [`Worker`](cicada::runtime::Worker) gets over HTTP.
//!
//! ```no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use std::time::Duration;
//! use cicada::runtime::Worker;
//! use cicada_client::ActorClient;
//! # struct CounterActor;
//! # impl cicada::Actor for CounterActor {
//! #     type Ref = Counter; type State = cicada::NoState;
//! #     async fn handle(&self, _: &mut cicada::Turn<Self>, _: Msg) -> cicada::HandlerResult { Ok(()) }
//! # }
//! # #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] struct Counter { id: String }
//! # impl cicada::ActorRef for Counter { const TYPE: &'static str = "counter"; type Message = Msg; }
//! # #[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)] #[serde(tag = "type")] enum Msg { Add }
//!
//! // No redirects (a redirect can send a POST again) and no retry policy of the client's own.
//! let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
//! let source = ActorClient::with_client("https://actors.internal", http).turn_timeout(Duration::from_secs(5));
//! let worker = Worker::builder(source).host(CounterActor).build();
//! worker.start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! **Requests.** [`register_contract`](TurnSource::register_contract) is `PUT /types/{name}`, and on a `409
//! contract-conflict` one `GET /types/{name}` for the registration that stays (a diagnostic, not a snapshot taken
//! with the conflict; a type gone by then is [`SourceError::Unavailable`]). The requested hash is computed from the
//! contract's name and schemas, not taken from a contract read from the registry. [`list_types`](TurnSource::list_types)
//! is `GET /types`, with each served hash and schema, descriptions included. Leasing, renewing, releasing and
//! completing are `POST /leases`, `/leases/renew`, `/leases/release` and `/leases/complete`: one request each, never
//! retried by this client whatever [`ActorClient::send_attempts`] says, and bounded by
//! [`ActorClient::turn_timeout`] from sending to the end of the response body.
//!
//! **Errors**, by the HTTP status of the response, never by the status a problem body claims: `400`, `413` and `422`
//! are [`SourceError::Invalid`] (also a URL or a request this client cannot build); `408`, `429`, `502`, `503`, `504`, a
//! transport failure, a body that cannot be read and the timeout are [`SourceError::Unavailable`], whose request may
//! still have been applied; any other status (a redirect, `401`, `403`, `404`, `405`, `409`, `500`, …) is
//! [`SourceError::Fatal`], never a lost lease, a contract mismatch or a rejection. Only `200` with a result this
//! version reads (`204` for a release) succeeds: an unknown result, outcome, cause or policy, or a missing field, is
//! `Fatal`.
//!
//! **The HTTP client.** The [`reqwest::Client`] keeps its authentication, TLS, timeouts, redirects and retries; this
//! client cannot see or change them. A default client follows redirects, and a `307` or `308` sends a POST again, so a
//! worker's client disables redirects, and any retry of its own, to keep one request per call.

use std::sync::Arc;

use async_trait::async_trait;
use cicada::ActorAddress;
use cicada::address::TypeName;
use cicada::api::leases::{
    CompleteTurnRequest, CompleteTurnResponse, LeaseTurnsRequest, LeaseTurnsResponse, ReleaseTurnRequest,
    RenewTurnRequest, RenewTurnResponse,
};
use cicada::api::{RegisterTypeRequest, TypeListView, TypeView, problem_types};
use cicada::contract::Contract;
use cicada::runtime::{
    CompleteTurnResult, LeaseTurnsResult, RegisterContractResult, RegisteredType, RenewTurnResult, SourceError,
    TurnReport, TurnSource,
};
use cicada::store::{MessageId, NodeId, TypeDef};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{ActorClient, Answer, ClientError, encode};

/// The error of a call that did not get an answer it can use.
fn client_error(what: &str, e: ClientError) -> SourceError {
    match e {
        ClientError::Invalid(why) => SourceError::Invalid(format!("{what}: {why}")),
        ClientError::Http(e) if e.is_builder() => SourceError::Invalid(format!("{what}: {e}")),
        ClientError::Http(e) => SourceError::Unavailable(format!("{what}: {e}")),
        e @ (ClientError::Api(_) | ClientError::Body(_)) => SourceError::Fatal(format!("{what}: {e}")),
    }
}

/// The error of an answer with a status the call does not expect, by that status alone.
fn unexpected(what: &str, answer: &Answer) -> SourceError {
    let problem = answer.problem();
    let detail = problem.detail.map(|d| format!(": {d}")).unwrap_or_default();
    let why = format!("{what}: HTTP {} {}{detail}", answer.status.as_u16(), problem.title);
    match answer.status.as_u16() {
        400 | 413 | 422 => SourceError::Invalid(why),
        408 | 429 | 502 | 503 | 504 => SourceError::Unavailable(why),
        _ => SourceError::Fatal(why),
    }
}

/// The body of an expected answer, which must decode.
fn decoded<T: DeserializeOwned>(what: &str, answer: &Answer) -> Result<T, SourceError> {
    answer.decode().map_err(|e| SourceError::Fatal(format!("{what}: {e}")))
}

/// The registered type a server served.
fn registered(view: TypeView) -> Result<RegisteredType, SourceError> {
    let definition = type_def(view)?;
    let contract = Contract::of(&definition).map_err(|e| SourceError::Fatal(format!("A served contract: {e}")))?;
    Ok(RegisteredType { definition, contract: Arc::new(contract) })
}

/// The registry entry of a served type: its schemas, descriptions included, and the hash the server serves.
fn type_def(view: TypeView) -> Result<TypeDef, SourceError> {
    let name = TypeName::parse(&view.name).map_err(|e| SourceError::Fatal(format!("A served type name: {e}")))?;
    let contract = Contract::new(name.clone(), view.state, view.messages).to_json();
    Ok(TypeDef { name, contract, contract_hash: view.hash })
}

impl ActorClient {
    /// One POST of `body` to `path`, bounded by the turn timeout, never repeated.
    async fn turn_call(&self, what: &str, path: &str, body: &impl Serialize) -> Result<Answer, SourceError> {
        let body = encode(body).map_err(|e| client_error(what, e))?;
        let url = self.url(path, &[]).map_err(|e| client_error(what, e))?;
        match tokio::time::timeout(self.turn_timeout, self.call(Method::POST, url, Some(body))).await {
            Ok(answer) => answer.map_err(|e| client_error(what, e)),
            Err(_) => {
                Err(SourceError::Unavailable(format!("{what}: no complete answer within {:?}", self.turn_timeout)))
            }
        }
    }

    /// The registration that stays after a conflict, read once.
    async fn existing(&self, name: &TypeName) -> Result<RegisteredType, SourceError> {
        const WHAT: &str = "Reading the conflicting registration";
        let url = self.url(&format!("/types/{name}"), &[]).map_err(|e| client_error(WHAT, e))?;
        let answer = self.call(Method::GET, url, None).await.map_err(|e| client_error(WHAT, e))?;
        match answer.status {
            StatusCode::OK => registered(decoded(WHAT, &answer)?),
            StatusCode::NOT_FOUND if answer.problem().problem_type == problem_types::NO_SUCH_TYPE => {
                Err(SourceError::Unavailable(format!("{WHAT}: type {name} is no longer registered")))
            }
            _ => Err(unexpected(WHAT, &answer)),
        }
    }
}

#[async_trait]
impl TurnSource for ActorClient {
    async fn register_contract(&self, contract: &Contract) -> Result<RegisterContractResult, SourceError> {
        const WHAT: &str = "Registering a contract";
        let name = contract.type_name();
        let request = RegisterTypeRequest {
            state: contract.state_schema().clone(),
            messages: contract.message_schemas().clone(),
        };
        let body = encode(&request).map_err(|e| client_error(WHAT, e))?;
        let url = self.url(&format!("/types/{name}"), &[]).map_err(|e| client_error(WHAT, e))?;
        let answer = self.call(Method::PUT, url, Some(body)).await.map_err(|e| client_error(WHAT, e))?;
        let problem_type = || answer.problem().problem_type;
        match answer.status {
            StatusCode::OK => Ok(RegisterContractResult::Registered(registered(decoded(WHAT, &answer)?)?)),
            StatusCode::UNPROCESSABLE_ENTITY if problem_type() == problem_types::INVALID_CONTRACT => {
                Ok(RegisterContractResult::InvalidContract(answer.problem().detail.unwrap_or_default()))
            }
            StatusCode::CONFLICT if problem_type() == problem_types::CONTRACT_CONFLICT => {
                // The server's hash of these schemas: a contract read from the registry keeps its stored hash.
                let requested_hash = Contract::new(name.clone(), request.state, request.messages).hash().to_owned();
                Ok(RegisterContractResult::Conflict { existing: self.existing(name).await?, requested_hash })
            }
            _ => Err(unexpected(WHAT, &answer)),
        }
    }

    async fn list_types(&self) -> Result<Vec<TypeDef>, SourceError> {
        const WHAT: &str = "Listing the types";
        let url = self.url("/types", &[]).map_err(|e| client_error(WHAT, e))?;
        let answer = self.call(Method::GET, url, None).await.map_err(|e| client_error(WHAT, e))?;
        if answer.status != StatusCode::OK {
            return Err(unexpected(WHAT, &answer));
        }
        decoded::<TypeListView>(WHAT, &answer)?.items.into_iter().map(type_def).collect()
    }

    async fn lease_turns(
        &self,
        owner: &NodeId,
        types: &std::collections::BTreeMap<TypeName, String>,
        limit: usize,
    ) -> Result<LeaseTurnsResult, SourceError> {
        const WHAT: &str = "Leasing turns";
        let request = LeaseTurnsRequest { owner: owner.clone(), types: types.clone(), limit };
        let answer = self.turn_call(WHAT, "/leases", &request).await?;
        if answer.status != StatusCode::OK {
            return Err(unexpected(WHAT, &answer));
        }
        Ok(decoded::<LeaseTurnsResponse>(WHAT, &answer)?.into())
    }

    async fn renew_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<RenewTurnResult, SourceError> {
        const WHAT: &str = "Renewing a lease";
        let answer = self.turn_call(WHAT, "/leases/renew", &RenewTurnRequest { actor: actor.clone(), epoch }).await?;
        if answer.status != StatusCode::OK {
            return Err(unexpected(WHAT, &answer));
        }
        Ok(decoded::<RenewTurnResponse>(WHAT, &answer)?.into())
    }

    async fn release_turn(&self, actor: &ActorAddress, epoch: i64) -> Result<(), SourceError> {
        const WHAT: &str = "Releasing a lease";
        let request = ReleaseTurnRequest { actor: actor.clone(), epoch };
        let answer = self.turn_call(WHAT, "/leases/release", &request).await?;
        if answer.status != StatusCode::NO_CONTENT {
            return Err(unexpected(WHAT, &answer));
        }
        Ok(())
    }

    async fn complete_turn(
        &self,
        actor: &ActorAddress,
        epoch: i64,
        message_id: &MessageId,
        report: TurnReport,
    ) -> Result<CompleteTurnResult, SourceError> {
        const WHAT: &str = "Completing a turn";
        let request =
            CompleteTurnRequest { actor: actor.clone(), epoch, message_id: message_id.clone(), report: report.into() };
        let answer = self.turn_call(WHAT, "/leases/complete", &request).await?;
        if answer.status != StatusCode::OK {
            return Err(unexpected(WHAT, &answer));
        }
        CompleteTurnResult::try_from(decoded::<CompleteTurnResponse>(WHAT, &answer)?)
            .map_err(|e| SourceError::Fatal(format!("{WHAT}: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn kind(e: &SourceError) -> &'static str {
        match e {
            SourceError::Invalid(_) => "invalid",
            SourceError::Unavailable(_) => "unavailable",
            SourceError::Fatal(_) => "fatal",
        }
    }

    /// conformance-fixtures, Scenario: Status classification (`fixtures/status.json` against the classifier itself).
    #[test]
    fn every_unexpected_status_is_classified_as_the_fixture_says_whatever_the_body_claims() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/status.json");
        let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(data["format"], 1);
        let statuses = |class: &str| -> Vec<u16> {
            data["unexpected"][class].as_array().unwrap().iter().map(|s| s.as_u64().unwrap() as u16).collect()
        };
        let (invalid, unavailable) = (statuses("invalid"), statuses("unavailable"));
        assert_eq!(invalid, [400, 413, 422]);
        assert_eq!(unavailable, [408, 429, 502, 503, 504]);
        let default = data["unexpected"]["default"].as_str().unwrap();
        assert_eq!(default, "fatal");
        for status in 100..=599u16 {
            let expected = if invalid.contains(&status) {
                "invalid"
            } else if unavailable.contains(&status) {
                "unavailable"
            } else {
                default
            };
            // A problem that claims a status of another class, one that claims none, and a body that is no problem.
            let claimed = if unavailable.contains(&status) { 400 } else { 503 };
            for text in [
                json!({"type": problem_types::NOT_FOUND, "title": "T", "status": claimed, "detail": "d"}).to_string(),
                json!({"type": problem_types::INTERNAL_ERROR, "title": "T"}).to_string(),
                "not a problem".to_owned(),
                String::new(),
            ] {
                let answer = Answer { status: StatusCode::from_u16(status).unwrap(), text };
                let error = unexpected("Calling", &answer);
                assert_eq!(kind(&error), expected, "HTTP {status} with {}: {error}", answer.text);
            }
        }
    }
}
