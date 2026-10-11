use std::collections::BTreeMap;

use axum::Router;
use axum::body::Body;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use cicada::address::RefBinding;
use cicada::api::{Problem, problem_types};
use cicada::time::parse_instant;
use futures::TryStreamExt;

use crate::actor::{ContainerActor, ContainerHandle, URI_LABEL};
use crate::engine::LogOptions;

/// Problem type of a Docker daemon that failed or could not be reached.
pub const DOCKER_UNAVAILABLE: &str = "urn:cicada:problem:docker-unavailable";

/// The logs router: `GET /containers/{name}/logs` streams the output of handle `name`'s container straight from Docker,
/// as `text/plain`. Nest it next to the actor API and wrap it in the same authentication.
///
/// | Query | Meaning | Default |
/// | --- | --- | --- |
/// | `tail` | The last N lines, or `all` | `100` |
/// | `follow` | Keep streaming new output | `false` |
/// | `since` | Only output since this RFC 3339 time | everything |
/// | `timestamps` | Prefix every line with its time | `false` |
///
/// Only containers labelled with the handle's URI are served; any other name is `404`. Errors are
/// `application/problem+json`.
///
/// ```no_run
/// # fn f(containers: cicada_docker::ContainerActor, system: cicada::ActorSystem) {
/// let api = cicada_axum::actor_api(system).merge(cicada_docker::container_logs(&containers));
/// let app = axum::Router::<()>::new().nest("/api", api);
/// # }
/// ```
pub fn container_logs(containers: &ContainerActor) -> Router {
    Router::new().route("/containers/{name}/logs", get(logs)).with_state(containers.clone())
}

async fn logs(
    State(containers): State<ContainerActor>,
    Path(name): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let instance = uri.path();
    let options = match log_options(&query) {
        Ok(options) => options,
        Err(detail) => {
            return problem(StatusCode::BAD_REQUEST, problem_types::BAD_REQUEST, "Bad request", detail, instance);
        }
    };
    let handle = ContainerHandle::new(name);
    let not_found =
        |detail: String| problem(StatusCode::NOT_FOUND, problem_types::NOT_FOUND, "Not found", detail, instance);
    let Ok(container) = containers.container_name(&handle) else {
        return not_found(format!("No container for handle '{}'", handle.name));
    };
    let handle_uri = RefBinding::<ContainerHandle>::new()
        .and_then(|b| b.to_address(&handle))
        .map(|a| a.to_string())
        .unwrap_or_default();
    match containers.engine().inspect(&container).await {
        Ok(Some(info)) if info.labels.get(URI_LABEL) == Some(&handle_uri) => {}
        Ok(_) => return not_found(format!("No container for {handle_uri}")),
        Err(e) => {
            return problem(StatusCode::BAD_GATEWAY, DOCKER_UNAVAILABLE, "Docker unavailable", e.to_string(), instance);
        }
    }
    let stream = containers.engine().logs(&container, options).map_err(std::io::Error::other);
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")],
        Body::from_stream(stream),
    )
        .into_response()
}

fn log_options(query: &BTreeMap<String, String>) -> Result<LogOptions, String> {
    let flag = |key: &str| match query.get(key).map(String::as_str) {
        None | Some("false") => Ok(false),
        Some("true" | "") => Ok(true),
        Some(other) => Err(format!("'{key}' must be true or false, was '{other}'")),
    };
    let tail = match query.get("tail").map(String::as_str) {
        None => Some(100),
        Some("all") => None,
        Some(n) => Some(n.parse::<u32>().map_err(|_| format!("'tail' must be a number or 'all', was '{n}'"))?),
    };
    let since = match query.get("since") {
        None => None,
        Some(text) => Some(parse_instant(text).map_err(|e| format!("'since' must be an RFC 3339 time: {e}"))?),
    };
    Ok(LogOptions { tail, follow: flag("follow")?, since, timestamps: flag("timestamps")? })
}

fn problem(status: StatusCode, problem_type: &str, title: &str, detail: String, instance: &str) -> Response {
    let problem = Problem {
        problem_type: problem_type.to_owned(),
        title: title.to_owned(),
        status: status.as_u16(),
        detail: Some(detail),
        instance: Some(instance.to_owned()),
    };
    (status, [(header::CONTENT_TYPE, "application/problem+json")], serde_json::to_string(&problem).unwrap_or_default())
        .into_response()
}
