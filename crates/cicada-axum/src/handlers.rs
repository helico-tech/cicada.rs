//! The route handlers. Each runs its work inside [`run`], which renders every failure, a panic included, as a problem.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cicada::api::{
    ActorView, CreateActorRequest, CreateActorResponse, HealthView, MessageChainView, MessageStatusView, MessageView,
    OutcomeView, PageView, SendRequest, SendResponse, TypeListView, TypeMatchView, TypeView, problem_types,
};
use cicada::store::{CancelResult, DeleteResult, MessageId};
use cicada::{ActorAddress, CreateActorResult, SendOptions, SendResult};
use futures::FutureExt;

use crate::Api;
use crate::problem::{ApiError, ApiProblem, json, panicked};
use crate::request::{self, Params, Target};

type Shared = State<Arc<Api>>;
type Outcome = Result<Response, ApiError>;

/// Runs a handler's work and renders its failure as a problem with `instance`.
async fn run(instance: String, work: impl Future<Output = Outcome>) -> Response {
    match AssertUnwindSafe(work).catch_unwind().await {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => e.render(&instance),
        Err(panic) => panicked(&instance, panic),
    }
}

fn no_such_instance(address: &ActorAddress) -> ApiProblem {
    ApiProblem::new(
        StatusCode::NOT_FOUND,
        problem_types::NO_SUCH_INSTANCE,
        "No such actor",
        format!("{address} does not exist"),
    )
}

fn no_such_message(id: &MessageId) -> ApiProblem {
    ApiProblem::new(
        StatusCode::NOT_FOUND,
        problem_types::NO_SUCH_MESSAGE,
        "No such message",
        format!("Message {id} is unknown or past retention"),
    )
}

fn no_such_type(address: &ActorAddress) -> ApiProblem {
    ApiProblem::new(
        StatusCode::NOT_FOUND,
        problem_types::NO_SUCH_TYPE,
        "No such actor type",
        format!("No registered actor type matches {address}"),
    )
}

fn unprocessable(problem_type: &'static str, title: &'static str, detail: impl Into<String>) -> ApiProblem {
    ApiProblem::new(StatusCode::UNPROCESSABLE_ENTITY, problem_type, title, detail)
}

// ---- /actors/{path} ----

pub(crate) async fn put_path(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::instance_path(req.uri())?;
        let body: CreateActorRequest = request::body(req, api.max_body).await?.unwrap_or_default();
        let response = |created| CreateActorResponse { uri: address.clone(), created };
        match api.system.create_actor(&address, body.state).await? {
            CreateActorResult::Created => Ok(json(StatusCode::CREATED, &response(true))),
            CreateActorResult::Exists(_) => Ok(json(StatusCode::OK, &response(false))),
            CreateActorResult::NoSuchType => Err(no_such_type(&address).into()),
            CreateActorResult::InvalidState(e) => {
                Err(unprocessable(problem_types::INVALID_STATE, "Invalid state", e).into())
            }
            CreateActorResult::InvalidAddress(e) => {
                Err(unprocessable(problem_types::INVALID_ADDRESS, "Invalid address", e).into())
            }
        }
    })
    .await
}

pub(crate) async fn get_path(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        match request::target(req.uri())? {
            Target::Instance(address) => {
                let snapshot = api.system.get_actor(&address).await?.ok_or_else(|| no_such_instance(&address))?;
                Ok(json(StatusCode::OK, &ActorView::from(snapshot)))
            }
            Target::Mailbox(address) => {
                let params = Params::of(req.uri())?;
                let page = api
                    .system
                    .pending(&address, params.cursor().as_ref(), params.limit()?)
                    .await?
                    .ok_or_else(|| no_such_instance(&address))?;
                Ok(json(StatusCode::OK, &PageView::of(page, MessageView::from)))
            }
        }
    })
    .await
}

pub(crate) async fn delete_path(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::instance_path(req.uri())?;
        match api.system.delete_actor(&address).await? {
            DeleteResult::Deleted { .. } => Ok(StatusCode::NO_CONTENT.into_response()),
            DeleteResult::Absent => Err(no_such_instance(&address).into()),
        }
    })
    .await
}

pub(crate) async fn post_path(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        match request::target(req.uri())? {
            Target::Mailbox(address) => send(&api, address, req).await,
            Target::Instance(_) => Err(ApiProblem {
                allow: Some("GET, PUT, DELETE"),
                ..ApiProblem::new(
                    StatusCode::METHOD_NOT_ALLOWED,
                    problem_types::METHOD_NOT_ALLOWED,
                    "Method not allowed",
                    "POST is only defined on /actors/{path}/messages",
                )
            }
            .into()),
        }
    })
    .await
}

async fn send(api: &Api, address: ActorAddress, req: Request) -> Outcome {
    let body: SendRequest = request::body(req, api.max_body)
        .await?
        .ok_or_else(|| ApiProblem::bad_request("A send needs a JSON body with a type"))?;
    let delivery = body.delivery().map_err(ApiProblem::bad_request)?;
    let id = body.id.unwrap_or_else(MessageId::random);
    let options = SendOptions {
        delivery,
        correlation_id: body.correlation_id,
        headers: body.headers,
        retry: body.retry,
        retention: body.retention,
        id: Some(id.clone()),
    };
    match api.system.send_message(&address, &body.message_type, body.payload, options).await? {
        SendResult::Accepted(m) => {
            Ok(json(StatusCode::ACCEPTED, &SendResponse { id: m.id, deliver_at: Some(m.deliver_at) }))
        }
        SendResult::Duplicate(existing) => {
            Ok(json(StatusCode::OK, &SendResponse { id, deliver_at: existing.map(|m| m.deliver_at) }))
        }
        SendResult::IdConflict(existing) => Err(ApiProblem::new(
            StatusCode::CONFLICT,
            problem_types::MESSAGE_ID_CONFLICT,
            "Message id in use",
            format!("Message id {id} was already accepted for a {} message to {}", existing.message_type, existing.to),
        )
        .into()),
        SendResult::InvalidHeaders(e) => {
            Err(unprocessable(problem_types::INVALID_HEADERS, "Invalid headers", e).into())
        }
        SendResult::NoSuchInstance => Err(no_such_instance(&address).into()),
        SendResult::UnknownType { message_type, template } => Err(unprocessable(
            problem_types::UNKNOWN_MESSAGE_TYPE,
            "Unknown message type",
            format!("{template} does not accept message type '{message_type}'"),
        )
        .into()),
        SendResult::InvalidPayload { message_type, error } => Err(unprocessable(
            problem_types::INVALID_PAYLOAD,
            "Invalid payload",
            format!("Payload of '{message_type}' cannot be decoded: {error}"),
        )
        .into()),
    }
}

// ---- listings ----

pub(crate) async fn list_actors(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let page = api.system.list_actors(&params.prefix(), params.cursor().as_ref(), params.limit()?).await?;
        Ok(json(StatusCode::OK, &PageView::of(page, |a| a)))
    })
    .await
}

pub(crate) async fn list_pending(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let page = api.system.list_pending(&params.prefix(), params.cursor().as_ref(), params.limit()?).await?;
        Ok(json(StatusCode::OK, &PageView::of(page, MessageView::from)))
    })
    .await
}

pub(crate) async fn failures(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let since = params.since()?;
        let page = api.system.failures(&params.prefix(), since, params.cursor().as_ref(), params.limit()?).await?;
        Ok(json(StatusCode::OK, &PageView::of(page, OutcomeView::from)))
    })
    .await
}

// ---- /messages/{id} ----

pub(crate) async fn message_status(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        let status = api.system.status(&id).await?.ok_or_else(|| no_such_message(&id))?;
        Ok(json(StatusCode::OK, &MessageStatusView::from(status)))
    })
    .await
}

pub(crate) async fn message_chain(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        let chain = api.system.chain(&id).await?.ok_or_else(|| no_such_message(&id))?;
        Ok(json(StatusCode::OK, &MessageChainView::from(chain)))
    })
    .await
}

pub(crate) async fn cancel_message(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        match api.system.cancel(&id).await? {
            CancelResult::Cancelled(_) => Ok(StatusCode::NO_CONTENT.into_response()),
            CancelResult::NotFound => Err(no_such_message(&id).into()),
            CancelResult::Running => Err(ApiProblem::new(
                StatusCode::CONFLICT,
                problem_types::MESSAGE_RUNNING,
                "Message is running",
                format!("Message {id} is being handled and cannot be cancelled"),
            )
            .into()),
            CancelResult::AlreadyFinished(outcome) => Err(ApiProblem::new(
                StatusCode::CONFLICT,
                problem_types::MESSAGE_FINISHED,
                "Message is finished",
                format!("Message {id} already has outcome {}", outcome.status),
            )
            .into()),
        }
    })
    .await
}

// ---- types and health ----

pub(crate) async fn types(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let Some(uri) = params.get("uri") else {
            let items = api.system.types().await?.iter().map(TypeView::from).collect();
            return Ok(json(StatusCode::OK, &TypeListView { items }));
        };
        let address = ActorAddress::parse(uri)
            .map_err(|_| ApiProblem::bad_request(format!("uri must be an actor URI: '{uri}'")))?;
        let resolved = api.system.type_of(&address).await?.ok_or_else(|| no_such_type(&address))?;
        Ok(json(StatusCode::OK, &TypeMatchView::from(&resolved)))
    })
    .await
}

pub(crate) async fn health(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let system = &api.system;
        if !system.health().await {
            return Err(ApiProblem::new(
                StatusCode::SERVICE_UNAVAILABLE,
                problem_types::UNHEALTHY,
                "Node unhealthy",
                format!(
                    "Node {} is {} or its store is unreachable",
                    system.node_id(),
                    system.state().name().to_lowercase()
                ),
            )
            .into());
        }
        let view = HealthView {
            status: "up".to_owned(),
            node: system.node_id().to_string(),
            state: system.state().name().to_owned(),
        };
        Ok(json(StatusCode::OK, &view))
    })
    .await
}
