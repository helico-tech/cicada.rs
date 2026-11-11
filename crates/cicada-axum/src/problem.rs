//! Problem responses (RFC 9457) and the mapping of failures to them.

use std::any::Any;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cicada::OpError;
use cicada::api::{Problem, problem_types};
use cicada::store::StoreError;
use serde::Serialize;

use crate::PROBLEM_JSON;

/// A problem response, returned by handlers and rendered with the request's URI as `instance`.
#[derive(Debug)]
pub(crate) struct ApiProblem {
    pub status: StatusCode,
    pub problem_type: &'static str,
    pub title: &'static str,
    pub detail: String,
    /// The `Allow` header of a `405`.
    pub allow: Option<&'static str>,
}

impl ApiProblem {
    pub fn new(status: StatusCode, problem_type: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self { status, problem_type, title, detail: detail.into(), allow: None }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, problem_types::BAD_REQUEST, "Bad request", detail)
    }

    pub fn too_large(limit: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            problem_types::REQUEST_TOO_LARGE,
            "Request too large",
            format!("The request body exceeds {limit} bytes"),
        )
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            problem_types::INTERNAL_ERROR,
            "Internal error",
            "The node failed to complete the request",
        )
    }

    pub fn render(self, instance: &str) -> Response {
        let problem = Problem {
            problem_type: self.problem_type.to_owned(),
            title: self.title.to_owned(),
            status: self.status.as_u16(),
            detail: Some(self.detail),
            instance: Some(instance.to_owned()),
        };
        let mut response = body(self.status, PROBLEM_JSON, &problem);
        if let Some(allow) = self.allow {
            response.headers_mut().insert(header::ALLOW, HeaderValue::from_static(allow));
        }
        response
    }
}

/// Why a handler failed: a problem it chose, or a failed operation of the node.
#[derive(Debug)]
pub(crate) enum ApiError {
    Problem(ApiProblem),
    Op(OpError),
}

impl From<ApiProblem> for ApiError {
    fn from(p: ApiProblem) -> Self {
        ApiError::Problem(p)
    }
}

impl From<OpError> for ApiError {
    fn from(e: OpError) -> Self {
        ApiError::Op(e)
    }
}

impl ApiError {
    /// Renders the failure: `400` for an invalid request (also data the store refuses), `503` when the store is
    /// unavailable, `500` for corrupt stored data and a node that cannot initialize.
    pub fn render(self, instance: &str) -> Response {
        let problem = match self {
            ApiError::Problem(p) => p,
            ApiError::Op(OpError::Invalid(detail)) | ApiError::Op(OpError::Store(StoreError::Invalid(detail))) => {
                ApiProblem::bad_request(detail)
            }
            ApiError::Op(OpError::Store(e @ StoreError::Unavailable(_))) => {
                tracing::warn!("Actor API {instance} failed: store unavailable: {e}");
                ApiProblem::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    problem_types::STORE_UNAVAILABLE,
                    "Store unavailable",
                    "The actor store could not complete the request",
                )
            }
            ApiError::Op(e) => {
                tracing::error!("Actor API {instance} failed unexpectedly: {e}");
                ApiProblem::internal()
            }
        };
        problem.render(instance)
    }
}

/// The `500` for a handler that panicked.
pub(crate) fn panicked(instance: &str, panic: Box<dyn Any + Send>) -> Response {
    let message = panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned());
    tracing::error!("Actor API {instance} failed unexpectedly: {message}");
    ApiProblem::internal().render(instance)
}

/// A JSON response.
pub(crate) fn json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    body(status, "application/json", value)
}

fn body<T: Serialize>(status: StatusCode, content_type: &'static str, value: &T) -> Response {
    let bytes = serde_json::to_vec(value).expect("wire types always serialize");
    (status, [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))], bytes).into_response()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The status and the body of `response`, which must be a problem.
    pub(crate) async fn problem_of(response: Response) -> (StatusCode, Problem) {
        let status = response.status();
        assert_eq!(response.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body is readable");
        (status, serde_json::from_slice(&bytes).expect("a problem"))
    }

    #[tokio::test]
    async fn the_store_errors_map_to_400_503_and_500() {
        let cases = [
            (StoreError::Invalid("out of range".into()), 400, problem_types::BAD_REQUEST),
            (StoreError::unavailable("closed"), 503, problem_types::STORE_UNAVAILABLE),
            (StoreError::Corrupt("stored state is corrupt".into()), 500, problem_types::INTERNAL_ERROR),
        ];
        for (error, status, problem_type) in cases {
            let (actual, problem) = problem_of(ApiError::Op(OpError::Store(error)).render("/actors/counter/x")).await;
            assert_eq!(actual.as_u16(), status);
            assert_eq!(problem.problem_type, problem_type);
            assert_eq!(problem.instance.as_deref(), Some("/actors/counter/x"));
        }
    }
}
