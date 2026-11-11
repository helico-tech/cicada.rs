//! The route handlers. Each runs its work inside [`run`], which renders every failure, a panic included, as a problem.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cicada::address::TypeName;
use cicada::api::{
    ActorView, BulkRequest, BulkResponse, CreateActorRequest, CreateActorResponse, HealthView, MailboxMessageView,
    MessageStatusView, MessageView, NodeDetailView, NodeListView, NodeView, OutcomeView, PageView, PurgeResponse,
    RunningTurnView, SendRequest, SendResponse, StatsView, TypeListView, TypeView, problem_types,
};
use cicada::runtime::{BulkResult, RetryResult};
use cicada::store::{CancelResult, DeleteResult, DeliverNowResult, MessageId, NodeId, RemoveTypeResult};
use cicada::{ActorAddress, CreateActorResult, SendOptions, SendResult};
use futures::FutureExt;

use crate::Api;
use crate::problem::{ApiError, ApiProblem, json, panicked};
use crate::request::{self, Params};

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

fn no_running_turn(address: &ActorAddress) -> ApiProblem {
    ApiProblem::new(
        StatusCode::NOT_FOUND,
        problem_types::NO_RUNNING_TURN,
        "No running turn",
        format!("No turn is running on {address}"),
    )
}

fn no_such_node(id: &str) -> ApiProblem {
    ApiProblem::new(
        StatusCode::NOT_FOUND,
        problem_types::NO_SUCH_NODE,
        "No such node",
        format!("No node {id} is known"),
    )
}

fn message_running(id: &MessageId) -> ApiProblem {
    ApiProblem::new(
        StatusCode::CONFLICT,
        problem_types::MESSAGE_RUNNING,
        "Message is running",
        format!("Message {id} is being handled"),
    )
}

fn message_finished(id: &MessageId, status: cicada::store::OutcomeStatus) -> ApiProblem {
    ApiProblem::new(
        StatusCode::CONFLICT,
        problem_types::MESSAGE_FINISHED,
        "Message is finished",
        format!("Message {id} already has outcome {status}"),
    )
}

fn unprocessable(problem_type: &'static str, title: &'static str, detail: impl Into<String>) -> ApiProblem {
    ApiProblem::new(StatusCode::UNPROCESSABLE_ENTITY, problem_type, title, detail)
}

// ---- /actors/{type}/{key} ----

pub(crate) async fn put_actor(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        let body: CreateActorRequest = request::body(req, api.max_body).await?.ok_or_else(|| {
            ApiProblem::bad_request(r#"A create needs a body with the initial state: {"state": ...}"#)
        })?;
        let response = |created| CreateActorResponse { address: address.clone(), created };
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

pub(crate) async fn get_actor(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        let snapshot = api.system.get_actor(&address).await?.ok_or_else(|| no_such_instance(&address))?;
        Ok(json(StatusCode::OK, &ActorView::from(snapshot)))
    })
    .await
}

pub(crate) async fn mailbox(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        let params = Params::of(req.uri())?;
        let page = api
            .system
            .mailbox(&address, params.cursor().as_ref(), params.limit()?)
            .await?
            .ok_or_else(|| no_such_instance(&address))?;
        Ok(json(StatusCode::OK, &PageView::of(page, MailboxMessageView::from)))
    })
    .await
}

pub(crate) async fn pause_mailbox(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        if !api.system.pause_mailbox(&address).await? {
            return Err(no_such_instance(&address).into());
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub(crate) async fn resume_mailbox(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        if !api.system.resume_mailbox(&address).await? {
            return Err(no_such_instance(&address).into());
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub(crate) async fn purge_mailbox(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        let purged = api.system.purge_mailbox(&address).await?.ok_or_else(|| no_such_instance(&address))?;
        Ok(json(StatusCode::OK, &PurgeResponse { purged }))
    })
    .await
}

pub(crate) async fn running_turn(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        let turn = api.system.running_turn(&address).await?.ok_or_else(|| no_running_turn(&address))?;
        Ok(json(StatusCode::OK, &RunningTurnView::from(turn)))
    })
    .await
}

pub(crate) async fn cancel_turn(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        if !api.system.cancel_turn(&address).await? {
            return Err(no_running_turn(&address).into());
        }
        Ok(StatusCode::ACCEPTED.into_response())
    })
    .await
}

pub(crate) async fn delete_actor(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        match api.system.delete_actor(&address).await? {
            DeleteResult::Deleted { .. } => Ok(StatusCode::NO_CONTENT.into_response()),
            DeleteResult::Absent => Err(no_such_instance(&address).into()),
        }
    })
    .await
}

pub(crate) async fn post_actor(req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        Err(ApiProblem {
            allow: Some("GET, PUT, DELETE"),
            ..ApiProblem::new(
                StatusCode::METHOD_NOT_ALLOWED,
                problem_types::METHOD_NOT_ALLOWED,
                "Method not allowed",
                "POST is only defined on /actors/{type}/{key}/messages",
            )
        }
        .into())
    })
    .await
}

pub(crate) async fn send_message(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let address = request::address(req.uri())?;
        send(&api, address, req).await
    })
    .await
}

async fn send(api: &Api, address: ActorAddress, req: Request) -> Outcome {
    let body: SendRequest = request::body(req, api.max_body)
        .await?
        .ok_or_else(|| ApiProblem::bad_request("A send needs a JSON body with a type"))?;
    let delivery = body.delivery().map_err(|e| ApiProblem::bad_request(e.to_string()))?;
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
        SendResult::NoSuchInstance => Err(no_such_instance(&address).into()),
        SendResult::UnknownType { message_type, type_name } => Err(unprocessable(
            problem_types::UNKNOWN_MESSAGE_TYPE,
            "Unknown message type",
            format!("{type_name} does not accept message type '{message_type}'"),
        )
        .into()),
        // The error names the message type and says why already.
        SendResult::InvalidPayload { error, .. } => {
            Err(unprocessable(problem_types::INVALID_PAYLOAD, "Invalid payload", error).into())
        }
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

pub(crate) async fn outcomes(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let (status, since) = (params.status()?, params.since()?);
        let page =
            api.system.outcomes(status, &params.prefix(), since, params.cursor().as_ref(), params.limit()?).await?;
        Ok(json(StatusCode::OK, &PageView::of(page, OutcomeView::from)))
    })
    .await
}

pub(crate) async fn list_turns(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        let node = params.node();
        let page = api
            .system
            .list_running_turns(node.as_ref(), &params.prefix(), params.cursor().as_ref(), params.limit()?)
            .await?;
        Ok(json(StatusCode::OK, &PageView::of(page, RunningTurnView::from)))
    })
    .await
}

pub(crate) async fn stats(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let params = Params::of(req.uri())?;
        Ok(json(StatusCode::OK, &StatsView::from(api.system.stats(&params.prefix()).await?)))
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

pub(crate) async fn caused(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        let params = Params::of(req.uri())?;
        let page = api
            .system
            .caused_by(&id, params.cursor().as_ref(), params.limit()?)
            .await?
            .ok_or_else(|| no_such_message(&id))?;
        Ok(json(StatusCode::OK, &PageView::of(page, MessageStatusView::from)))
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
            CancelResult::Running => Err(message_running(&id).into()),
            CancelResult::AlreadyFinished(outcome) => Err(message_finished(&id, outcome.status).into()),
        }
    })
    .await
}

pub(crate) async fn retry_message(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        match api.system.retry_message(&id).await? {
            RetryResult::Accepted(m) => {
                Ok(json(StatusCode::ACCEPTED, &SendResponse { id: m.id, deliver_at: Some(m.deliver_at) }))
            }
            RetryResult::NotFound => Err(no_such_message(&id).into()),
            RetryResult::NoSuchInstance(address) => Err(no_such_instance(&address).into()),
            RetryResult::NotRetryable { state } => Err(ApiProblem::new(
                StatusCode::CONFLICT,
                problem_types::MESSAGE_NOT_RETRYABLE,
                "Message cannot be retried",
                format!("Message {id} is {state}; only failed or cancelled messages can be retried"),
            )
            .into()),
        }
    })
    .await
}

pub(crate) async fn deliver_now(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let id = request::message_id(&id)?;
        match api.system.deliver_now(&id).await? {
            DeliverNowResult::Delivered(m) => Ok(json(StatusCode::OK, &MessageView::from(m))),
            DeliverNowResult::NotFound => Err(no_such_message(&id).into()),
            DeliverNowResult::Running => Err(message_running(&id).into()),
            DeliverNowResult::AlreadyFinished(outcome) => Err(message_finished(&id, outcome.status).into()),
        }
    })
    .await
}

async fn bulk(
    api: &Api,
    req: Request,
    act: impl AsyncFnOnce(cicada::runtime::BulkSelection) -> Result<BulkResult, cicada::OpError>,
) -> Outcome {
    let body: BulkRequest = request::body(req, api.max_body)
        .await?
        .ok_or_else(|| ApiProblem::bad_request("A bulk action needs a JSON body with ids or a filter"))?;
    let selection = body.selection().map_err(|e| ApiProblem::bad_request(e.to_string()))?;
    Ok(json(StatusCode::OK, &BulkResponse::from(act(selection).await?)))
}

pub(crate) async fn bulk_cancel(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move { bulk(&api, req, async |s| api.system.bulk_cancel(s).await).await }).await
}

pub(crate) async fn bulk_retry(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move { bulk(&api, req, async |s| api.system.bulk_retry(s).await).await }).await
}

// ---- /nodes ----

pub(crate) async fn nodes(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let items = api.system.nodes().await?.iter().map(NodeView::from).collect();
        Ok(json(StatusCode::OK, &NodeListView { items }))
    })
    .await
}

pub(crate) async fn node(State(api): Shared, Path(id): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let node_id = NodeId::new(&id).map_err(|_| no_such_node(&id))?;
        let node = api.system.node(&node_id).await?.ok_or_else(|| no_such_node(&id))?;
        Ok(json(StatusCode::OK, &NodeDetailView::from(node)))
    })
    .await
}

// ---- types and health ----

pub(crate) async fn types(State(api): Shared, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let items = api.system.types().await?.iter().map(TypeView::from).collect();
        Ok(json(StatusCode::OK, &TypeListView { items }))
    })
    .await
}

pub(crate) async fn type_by_name(State(api): Shared, Path(name): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let found = api.system.types().await?.into_iter().find(|t| t.name().as_str() == name);
        let found = found.ok_or_else(|| {
            ApiProblem::new(
                StatusCode::NOT_FOUND,
                problem_types::NO_SUCH_TYPE,
                "No such actor type",
                format!("No registered actor type {name}"),
            )
        })?;
        Ok(json(StatusCode::OK, &TypeView::from(&found)))
    })
    .await
}

pub(crate) async fn remove_type(State(api): Shared, Path(name): Path<String>, req: Request) -> Response {
    let instance = request::instance(&req);
    run(instance, async move {
        let absent = || {
            ApiProblem::new(
                StatusCode::NOT_FOUND,
                problem_types::NO_SUCH_TYPE,
                "No such actor type",
                format!("No registered actor type {name}"),
            )
        };
        // A name that is not a valid type name cannot be registered.
        let type_name = TypeName::parse(&name).map_err(|_| absent())?;
        match api.system.remove_type(&type_name).await? {
            RemoveTypeResult::Removed => Ok(StatusCode::NO_CONTENT.into_response()),
            RemoveTypeResult::Absent => Err(absent().into()),
            RemoveTypeResult::InUse => Err(ApiProblem::new(
                StatusCode::CONFLICT,
                problem_types::TYPE_IN_USE,
                "Type is in use",
                format!("Actors of type {name} exist; delete them first"),
            )
            .into()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::tests::problem_of;

    async fn panicking() -> Outcome {
        panic!("a bug")
    }

    #[tokio::test]
    async fn a_handler_that_panics_answers_500_with_its_instance() {
        let (status, problem) = problem_of(run("/actors/counter/x".to_owned(), panicking()).await).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(problem.problem_type, problem_types::INTERNAL_ERROR);
        assert_eq!(problem.instance.as_deref(), Some("/actors/counter/x"));
    }
}
