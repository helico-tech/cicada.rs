//! Request parsing: the raw actor path, query parameters and bodies.

use axum::extract::{OriginalUri, Query, Request};
use axum::http::{Uri, header};
use cicada::ActorAddress;
use cicada::store::{MessageId, Page, PageCursor};
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

/// What a `/actors/{path}` request addresses.
pub(crate) enum Target {
    Instance(ActorAddress),
    Mailbox(ActorAddress),
}

/// The raw (still percent-encoded) segments below `/actors/`, empty ones dropped.
fn raw_segments(uri: &Uri) -> Vec<&str> {
    let path = uri.path();
    let below = path.strip_prefix("/actors/").unwrap_or(path);
    below.split('/').filter(|s| !s.is_empty()).collect()
}

/// The instance the whole path names (`PUT`, `DELETE`).
pub(crate) fn instance_path(uri: &Uri) -> Result<ActorAddress, ApiProblem> {
    address_of(&raw_segments(uri))
}

/// Instance or mailbox (`GET`, `POST`): a trailing raw segment `messages` is the mailbox. The escaped `%6Dessages`
/// decodes to the same text but is an actor segment.
pub(crate) fn target(uri: &Uri) -> Result<Target, ApiProblem> {
    let raw = raw_segments(uri);
    match raw.split_last() {
        Some((&last, rest)) if !rest.is_empty() && last == ActorAddress::MAILBOX_SEGMENT => {
            Ok(Target::Mailbox(address_of(rest)?))
        }
        _ => Ok(Target::Instance(address_of(&raw)?)),
    }
}

fn address_of(raw: &[&str]) -> Result<ActorAddress, ApiProblem> {
    if raw.is_empty() {
        return Err(ApiProblem::bad_request("An actor path needs at least one segment"));
    }
    let segments = raw.iter().map(|s| decode_segment(s)).collect::<Result<Vec<_>, _>>()?;
    ActorAddress::of(segments).map_err(|e| ApiProblem::bad_request(e.to_string()))
}

/// Decodes the `%XX` escapes of a path segment; every other character stands for itself, as in any HTTP path.
fn decode_segment(raw: &str) -> Result<String, ApiProblem> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |b: u8| (b as char).to_digit(16);
            match (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
                (Some(hi), Some(lo)) => out.push((hi * 16 + lo) as u8),
                _ => return Err(ApiProblem::bad_request(format!("Invalid percent escape in path segment '{raw}'"))),
            }
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .map_err(|_| ApiProblem::bad_request(format!("Percent escapes in path segment '{raw}' are not valid UTF-8")))
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

    /// The `prefix` as an actor-URI prefix; the scheme may be left out.
    pub fn prefix(&self) -> String {
        match self.get("prefix") {
            None | Some("") => String::new(),
            Some(p) if p.starts_with(ActorAddress::SCHEME) => p.to_owned(),
            Some(p) => format!("{}{p}", ActorAddress::SCHEME),
        }
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

    fn target_of(path: &str) -> Result<Target, ApiProblem> {
        target(&path.parse::<Uri>().unwrap())
    }

    #[test]
    fn mailbox_only_for_a_raw_trailing_messages_segment() {
        assert!(matches!(target_of("/actors/a/b/messages"), Ok(Target::Mailbox(a)) if a.as_str() == "actor://a/b"));
        assert!(
            matches!(target_of("/actors/a/%6Dessages"), Ok(Target::Instance(a)) if a.as_str() == "actor://a/messages")
        );
        assert!(matches!(target_of("/actors/messages"), Ok(Target::Instance(a)) if a.as_str() == "actor://messages"));
        assert!(matches!(target_of("/actors/a//b/"), Ok(Target::Instance(a)) if a.as_str() == "actor://a/b"));
    }

    #[test]
    fn segments_decode_leniently_and_canonically() {
        let a = instance_path(&"/actors/c/a%20b%2fc%c3%a9".parse().unwrap()).unwrap();
        assert_eq!(a.as_str(), "actor://c/a%20b%2Fc%C3%A9");
        let dots = instance_path(&"/actors/c/%2E%2E/./x:y".parse().unwrap()).unwrap();
        assert_eq!(dots.segments(), ["c", "..", ".", "x:y"]);
        assert!(instance_path(&"/actors/c/%zz".parse().unwrap()).is_err());
        assert!(instance_path(&"/actors/c/%C3".parse().unwrap()).is_err());
    }
}
