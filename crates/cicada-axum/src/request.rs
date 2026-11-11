//! Request parsing: the raw actor path, query parameters and bodies.

use axum::extract::{OriginalUri, Query, Request};
use axum::http::{Uri, header};
use cicada::ActorAddress;
use cicada::store::{MessageId, NodeId, OutcomeStatus, Page, PageCursor};
use cicada::time::{MAX_INSTANT, Timestamp, format_instant, parse_instant};
use futures::StreamExt;
use serde::de::DeserializeOwned;

use crate::MAX_PAGE_SIZE;
use crate::problem::ApiProblem;

/// The request as the client sent it (path and query), also when the API is nested under a prefix: the problem
/// `instance`.
pub(crate) fn instance(req: &Request) -> String {
    let uri = req.extensions().get::<OriginalUri>().map(|o| &o.0).unwrap_or(req.uri());
    uri.path_and_query().map(|pq| pq.as_str().to_owned()).unwrap_or_else(|| uri.path().to_owned())
}

/// The actor a `/actors/{type}/{key}` request addresses. The two segments are read raw (still percent-encoded), so
/// that an encoded `/` in a name is not mistaken for the end of the key.
pub(crate) fn address(uri: &Uri) -> Result<ActorAddress, ApiProblem> {
    let path = uri.path();
    let below = path.strip_prefix("/actors/").unwrap_or(path);
    let mut segments = below.split('/');
    match (segments.next(), segments.next()) {
        (Some(type_name), Some(key)) => {
            ActorAddress::parse(&format!("{type_name}/{key}")).map_err(|e| ApiProblem::bad_request(e.to_string()))
        }
        _ => Err(ApiProblem::bad_request("An actor path is /actors/{type}/{key}")),
    }
}

/// A message id path parameter.
pub(crate) fn message_id(raw: &str) -> Result<MessageId, ApiProblem> {
    MessageId::new(raw).map_err(|_| ApiProblem::bad_request("Missing message id"))
}

/// The decoded query parameters.
pub(crate) struct Params(Vec<(String, String)>);

impl Params {
    pub fn of(uri: &Uri) -> Result<Self, ApiProblem> {
        Query::<Vec<(String, String)>>::try_from_uri(uri)
            .map(|q| Params(q.0))
            .map_err(|e| ApiProblem::bad_request(format!("Malformed query: {e}")))
    }

    /// The first value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The `node` filter of a listing; an absent or blank value lists every node.
    pub fn node(&self) -> Option<NodeId> {
        self.get("node").and_then(|n| NodeId::new(n).ok())
    }

    /// The `prefix` of the addresses to list; empty for all.
    pub fn prefix(&self) -> String {
        self.get("prefix").unwrap_or_default().to_owned()
    }

    pub fn cursor(&self) -> Option<PageCursor> {
        self.get("cursor").filter(|c| !c.is_empty()).map(|c| PageCursor(c.to_owned()))
    }

    pub fn limit(&self) -> Result<usize, ApiProblem> {
        let Some(raw) = self.get("limit") else {
            return Ok(Page::<()>::DEFAULT_SIZE);
        };
        raw.parse::<usize>()
            .ok()
            .filter(|l| (1..=MAX_PAGE_SIZE).contains(l))
            .ok_or_else(|| ApiProblem::bad_request(format!("limit must be a number in 1..{MAX_PAGE_SIZE}: '{raw}'")))
    }

    /// The `status` of the outcomes to list; `None` for all.
    pub fn status(&self) -> Result<Option<OutcomeStatus>, ApiProblem> {
        let Some(raw) = self.get("status") else {
            return Ok(None);
        };
        OutcomeStatus::from_wire_name(raw).map(Some).ok_or_else(|| {
            ApiProblem::bad_request(format!("status must be done, failed, cancelled or purged: '{raw}'"))
        })
    }

    /// `since`, within the range every backend stores.
    pub fn since(&self) -> Result<Option<Timestamp>, ApiProblem> {
        let Some(raw) = self.get("since") else {
            return Ok(None);
        };
        let since = parse_instant(raw)
            .map_err(|_| ApiProblem::bad_request(format!("since must be an ISO-8601 instant: '{raw}'")))?;
        if since > MAX_INSTANT {
            return Err(ApiProblem::bad_request(format!(
                "since must lie between -100000-01-01T00:00:00Z and {}: '{raw}'",
                format_instant(MAX_INSTANT)
            )));
        }
        Ok(Some(since))
    }
}

/// Decodes the request body strictly (see [`cicada::api`]); `None` for an empty body. A body larger than `limit`
/// bytes is refused with `413`: by its `Content-Length` when given, otherwise while reading, which stops one byte past
/// the limit.
pub(crate) async fn body<T: DeserializeOwned>(req: Request, limit: usize) -> Result<Option<T>, ApiProblem> {
    let declared = req.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|len| len > limit as u64) {
        return Err(ApiProblem::too_large(limit));
    }
    let mut stream = req.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiProblem::bad_request(format!("Unreadable request body: {e}")))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > limit {
            return Err(ApiProblem::too_large(limit));
        }
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| ApiProblem::bad_request(format!("Unreadable request body: not UTF-8: {e}")))?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let malformed = |e: serde_json::Error| ApiProblem::bad_request(format!("Malformed JSON body: {e}"));
    // Every request body is an object; serde's derived structs would also take an array of the fields.
    let value: serde_json::Value = serde_json::from_str(&text).map_err(malformed)?;
    if !value.is_object() {
        return Err(ApiProblem::bad_request(format!("Malformed JSON body: expected a JSON object, was {text}")));
    }
    T::deserialize(value).map(Some).map_err(malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address_of(path: &str) -> Result<ActorAddress, ApiProblem> {
        address(&path.parse::<Uri>().unwrap())
    }

    #[test]
    fn the_first_two_segments_are_the_address() {
        assert_eq!(address_of("/actors/a/b").unwrap().as_str(), "a/b");
        assert_eq!(address_of("/actors/a/b/messages").unwrap().as_str(), "a/b");
        assert_eq!(address_of("/actors/a/messages").unwrap().key(), "messages");
    }

    #[test]
    fn keys_stay_canonical_and_a_colon_is_part_of_the_name() {
        assert_eq!(address_of("/actors/c/a%20b%2fc%c3%a9").unwrap().as_str(), "c/a%20b%2Fc%C3%A9");
        assert_eq!(address_of("/actors/c/x%3Ay").unwrap().as_str(), "c/x:y");
        assert_eq!(address_of("/actors/c/x:y").unwrap().name(), "x:y");
        for bad in ["/actors/c/%zz", "/actors/c/%C3", "/actors/c/%2E%2E", "/actors/C/x"] {
            assert!(address_of(bad).is_err(), "{bad}");
        }
    }
}
