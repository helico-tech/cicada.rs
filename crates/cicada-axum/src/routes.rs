//! The routes of the API and the OpenAPI document generated from them.
//!
//! The handlers read their requests by hand, so that every failure is a problem+json answer. Each route therefore states
//! its inputs and responses here; the types below exist only to describe them.

use std::sync::Arc;

use aide::axum::ApiRouter;
use aide::axum::routing::{get_with, post_with};
use aide::generate::GenContext;
use aide::openapi::{Info, OpenApi, Operation, Response};
use aide::operation::OperationOutput;
use aide::transform::TransformOperation;
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query};
use axum::routing::{get, post};
use cicada::api::{
    ActorView, BulkRequest, BulkResponse, CreateActorRequest, CreateActorResponse, HealthView, MailboxMessageView,
    MessageStatusView, MessageView, NodeDetailView, NodeListView, OutcomeView, PageView, Problem, PurgeResponse,
    RunningTurnView, SendRequest, SendResponse, StatsView, TypeListView, TypeView,
};
use schemars::JsonSchema;

use crate::{Api, PROBLEM_JSON, handlers};

/// What a router allows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Only routes that read.
    Read,
    /// Every route.
    Full,
}

/// A problem+json answer, the form of every error.
struct ProblemJson;

impl OperationOutput for ProblemJson {
    type Inner = Problem;

    fn operation_response(ctx: &mut GenContext, operation: &mut Operation) -> Option<Response> {
        let mut response = Json::<Problem>::operation_response(ctx, operation)?;
        let media = response.content.shift_remove("application/json")?;
        response.content.insert(PROBLEM_JSON.to_owned(), media);
        response.description = "An error (RFC 9457).".to_owned();
        Some(response)
    }
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct ActorPath {
    /// The type name, e.g. `sync`.
    r#type: String,
    /// The key, e.g. `acme`, `b1:7`, or `_` for a singleton.
    key: String,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct MessagePath {
    /// The message id.
    id: String,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct TypePath {
    /// The type name, e.g. `sync`.
    name: String,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct NodePath {
    /// The node id.
    node: String,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct TurnsQuery {
    /// Only turns held by this node.
    node: Option<String>,
    /// A prefix of the address: `sync/` is one type, `sync/ac` a key prefix.
    prefix: Option<String>,
    /// The `next` of the previous page.
    cursor: Option<String>,
    /// Items per page, 1 to 1000; 100 by default.
    limit: Option<u32>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct PageQuery {
    /// The `next` of the previous page.
    cursor: Option<String>,
    /// Items per page, 1 to 1000; 100 by default.
    limit: Option<u32>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct ListQuery {
    /// A prefix of the address: `sync/` is one type, `sync/ac` a key prefix.
    prefix: Option<String>,
    /// The `next` of the previous page.
    cursor: Option<String>,
    /// Items per page, 1 to 1000; 100 by default.
    limit: Option<u32>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct StatsQuery {
    /// A prefix of the address: `sync/` is one type, `sync/ac` a key prefix.
    prefix: Option<String>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct OutcomesQuery {
    /// Only outcomes of this status: `done`, `failed`, `cancelled` or `purged`; all by default.
    status: Option<String>,
    /// A prefix of the address: `sync/` is one type, `sync/ac` a key prefix.
    prefix: Option<String>,
    /// Only messages that finished at or after this instant.
    since: Option<String>,
    /// The `next` of the previous page.
    cursor: Option<String>,
    /// Items per page, 1 to 1000; 100 by default.
    limit: Option<u32>,
}

fn described<'a>(op: TransformOperation<'a>, id: &str, summary: &str) -> TransformOperation<'a> {
    op.id(id).summary(summary).default_response::<ProblemJson>()
}

fn no_content<'a>(op: TransformOperation<'a>, description: &str) -> TransformOperation<'a> {
    op.response_with::<204, (), _>(|r| r.description(description))
}

fn routes(access: Access) -> ApiRouter<Arc<Api>> {
    let full = access == Access::Full;

    let mut actor = get_with(handlers::get_actor, |op| {
        described(op, "getActor", "One actor: its type, key values, state and version.")
            .input::<Path<ActorPath>>()
            .response::<200, Json<ActorView>>()
    });
    let mailbox = get_with(handlers::mailbox, |op| {
        described(
            op,
            "listMailbox",
            "The pending messages of one actor, in delivery order, each marked running or pending.",
        )
        .input::<(Path<ActorPath>, Query<PageQuery>)>()
        .response::<200, Json<PageView<MailboxMessageView>>>()
    });
    let message = get_with(handlers::message_status, |op| {
        described(op, "getMessage", "A message: pending, running or finished.")
            .input::<Path<MessagePath>>()
            .response::<200, Json<MessageStatusView>>()
    });
    let mut type_by_name = get_with(handlers::type_by_name, |op| {
        described(op, "getType", "One registered type with its JSON Schemas.")
            .input::<Path<TypePath>>()
            .response::<200, Json<TypeView>>()
    });
    if full {
        type_by_name = type_by_name.delete_with(handlers::remove_type, |op| {
            no_content(
                described(
                    op,
                    "removeType",
                    "Removes a type's registration. Refused while actors of the type exist (409). A node that hosts \
                     the type registers it again at its next start.",
                )
                .input::<Path<TypePath>>(),
                "Removed.",
            )
        });
        actor = actor
            .put_with(handlers::put_actor, |op| {
                described(
                    op,
                    "createActor",
                    "Creates an actor with its initial state. An existing one is left untouched.",
                )
                .input::<(Path<ActorPath>, Json<CreateActorRequest>)>()
                .response_with::<201, Json<CreateActorResponse>, _>(|r| r.description("Created."))
                .response_with::<200, Json<CreateActorResponse>, _>(|r| r.description("It already existed."))
            })
            .delete_with(handlers::delete_actor, |op| {
                no_content(
                    described(op, "deleteActor", "Deletes an actor and purges its mailbox.").input::<Path<ActorPath>>(),
                    "Deleted.",
                )
            });
    }

    let router = ApiRouter::new()
        .api_route(
            "/actors",
            get_with(handlers::list_actors, |op| {
                described(op, "listActors", "Actor addresses, by prefix.")
                    .input::<Query<ListQuery>>()
                    .response::<200, Json<PageView<cicada::ActorAddress>>>()
            }),
        )
        .api_route("/actors/{type}/{key}", actor)
        .api_route("/actors/{type}/{key}/mailbox", mailbox)
        .api_route(
            "/actors/{type}/{key}/turn",
            get_with(handlers::running_turn, |op| {
                described(op, "getTurn", "The turn running now on one actor; 404 when there is none.")
                    .input::<Path<ActorPath>>()
                    .response::<200, Json<RunningTurnView>>()
            }),
        )
        .api_route(
            "/messages",
            get_with(handlers::list_pending, |op| {
                described(op, "listMessages", "The pending messages of every actor under a prefix.")
                    .input::<Query<ListQuery>>()
                    .response::<200, Json<PageView<MessageView>>>()
            }),
        )
        .api_route("/messages/{id}", message)
        .api_route(
            "/messages/{id}/caused",
            get_with(handlers::caused, |op| {
                described(op, "listCaused", "The messages the turn that handled a message sent, one level down.")
                    .input::<(Path<MessagePath>, Query<PageQuery>)>()
                    .response::<200, Json<PageView<MessageStatusView>>>()
            }),
        )
        .api_route(
            "/outcomes",
            get_with(handlers::outcomes, |op| {
                described(op, "listOutcomes", "Finished messages with envelope and error, by status.")
                    .input::<Query<OutcomesQuery>>()
                    .response::<200, Json<PageView<OutcomeView>>>()
            }),
        )
        .api_route(
            "/stats",
            get_with(handlers::stats, |op| {
                described(op, "getStats", "The number of actors and pending messages, and the oldest due message.")
                    .input::<Query<StatsQuery>>()
                    .response::<200, Json<StatsView>>()
            }),
        )
        .api_route(
            "/nodes",
            get_with(handlers::nodes, |op| {
                described(op, "listNodes", "Every node with its status and the number of its running turns.")
                    .response::<200, Json<NodeListView>>()
            }),
        )
        .api_route(
            "/nodes/{node}",
            get_with(handlers::node, |op| {
                described(op, "getNode", "One node with its status and its running turns.")
                    .input::<Path<NodePath>>()
                    .response::<200, Json<NodeDetailView>>()
            }),
        )
        .api_route(
            "/turns",
            get_with(handlers::list_turns, |op| {
                described(op, "listTurns", "The turns running now, optionally of one node or under a prefix.")
                    .input::<Query<TurnsQuery>>()
                    .response::<200, Json<PageView<RunningTurnView>>>()
            }),
        )
        .api_route(
            "/types",
            get_with(handlers::types, |op| {
                described(op, "listTypes", "Every registered type with its JSON Schemas.")
                    .response::<200, Json<TypeListView>>()
            }),
        )
        .api_route("/types/{name}", type_by_name)
        .api_route(
            "/health",
            get_with(handlers::health, |op| {
                described(op, "getHealth", "Whether the node runs and its store answers.")
                    .response::<200, Json<HealthView>>()
            }),
        );
    if full { write_routes(router).route("/actors/{type}/{key}", post(handlers::post_actor)) } else { router }
}

/// The routes that change something: only on the full router.
fn write_routes(router: ApiRouter<Arc<Api>>) -> ApiRouter<Arc<Api>> {
    router
        .api_route(
            "/actors/{type}/{key}/messages",
            post_with(handlers::send_message, |op| {
                described(op, "sendMessage", "Sends a message. Idempotent on its id.")
                    .input::<(Path<ActorPath>, Json<SendRequest>)>()
                    .response_with::<202, Json<SendResponse>, _>(|r| r.description("Accepted."))
                    .response_with::<200, Json<SendResponse>, _>(|r| r.description("That id was already accepted."))
            }),
        )
        .api_route(
            "/actors/{type}/{key}/turn/cancel",
            post_with(handlers::cancel_turn, |op| {
                described(op, "cancelTurn", "Asks the owning node to cancel the running turn, within one heartbeat.")
                    .input::<Path<ActorPath>>()
                    .response_with::<202, (), _>(|r| r.description("The request is recorded."))
            }),
        )
        .api_route(
            "/actors/{type}/{key}/mailbox/pause",
            post_with(handlers::pause_mailbox, |op| {
                no_content(
                    described(op, "pauseMailbox", "Pauses an actor: messages queue up, no new turn is started.")
                        .input::<Path<ActorPath>>(),
                    "Paused.",
                )
            }),
        )
        .api_route(
            "/actors/{type}/{key}/mailbox/resume",
            post_with(handlers::resume_mailbox, |op| {
                no_content(
                    described(op, "resumeMailbox", "Resumes a paused actor.").input::<Path<ActorPath>>(),
                    "Resumed.",
                )
            }),
        )
        .api_route(
            "/actors/{type}/{key}/mailbox/purge",
            post_with(handlers::purge_mailbox, |op| {
                described(
                    op,
                    "purgeMailbox",
                    "Gives outcome purged to every pending message except one being handled; the actor stays.",
                )
                .input::<Path<ActorPath>>()
                .response::<200, Json<PurgeResponse>>()
            }),
        )
        .api_route(
            "/messages/bulk-cancel",
            post_with(handlers::bulk_cancel, |op| {
                described(op, "bulkCancelMessages", "Cancels messages by ids or by filter, at most 1000 per call.")
                    .input::<Json<BulkRequest>>()
                    .response::<200, Json<BulkResponse>>()
            }),
        )
        .api_route(
            "/messages/bulk-retry",
            post_with(handlers::bulk_retry, |op| {
                described(op, "bulkRetryMessages", "Retries messages by ids or by filter, at most 1000 per call.")
                    .input::<Json<BulkRequest>>()
                    .response::<200, Json<BulkResponse>>()
            }),
        )
        .api_route(
            "/messages/{id}/cancel",
            post_with(handlers::cancel_message, |op| {
                no_content(
                    described(op, "cancelMessage", "Cancels a pending message.").input::<Path<MessagePath>>(),
                    "Cancelled.",
                )
            }),
        )
        .api_route(
            "/messages/{id}/retry",
            post_with(handlers::retry_message, |op| {
                described(
                    op,
                    "retryMessage",
                    "Sends a failed or cancelled message again: a new message with a new id, caused by the original.",
                )
                .input::<Path<MessagePath>>()
                .response_with::<202, Json<SendResponse>, _>(|r| r.description("Accepted."))
            }),
        )
        .api_route(
            "/messages/{id}/deliver-now",
            post_with(handlers::deliver_now, |op| {
                described(op, "deliverMessageNow", "Makes a pending message due now, at a new place in delivery order.")
                    .input::<Path<MessagePath>>()
                    .response::<200, Json<MessageView>>()
            }),
        )
}

fn empty_document() -> OpenApi {
    OpenApi {
        info: Info {
            title: "Cicada actor API".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        ..OpenApi::default()
    }
}

/// The router of `access` with its document at `GET /openapi.json`.
pub(crate) fn router(access: Access, api: Arc<Api>) -> Router {
    let mut document = empty_document();
    let router = routes(access).finish_api(&mut document);
    let document = Arc::new(document);
    router.route("/openapi.json", get(move || async move { Json(document.as_ref().clone()) })).with_state(api)
}

/// The OpenAPI document of the full API.
pub fn openapi_document() -> serde_json::Value {
    let mut document = empty_document();
    let _: Router<Arc<Api>> = routes(Access::Full).finish_api(&mut document);
    serde_json::to_value(&document).expect("an OpenAPI document serializes")
}
