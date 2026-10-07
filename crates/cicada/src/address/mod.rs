//! Actor addresses: URIs such as `actor://sync/acme`, the URI templates actor types own, and typed refs.

mod actor_ref;
mod params;
pub mod percent;
mod template;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use actor_ref::{ActorRef, RefBinding};
pub use params::{from_params, to_params};
pub use template::{Segment, UriTemplate};

/// A malformed actor URI, template or path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AddressError(String);

impl AddressError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The URI of one actor instance, e.g. `actor://sync/acme`, always held in canonical form.
///
/// Form: `actor://<segment>/<segment>/…` with at least one segment. Each segment is non-empty and uses
/// `A-Z a-z 0-9 . _ ~ -`; any other character is percent-encoded (see [`percent`]). Two addresses are equal exactly
/// when their canonical strings are equal, so `actor://a/%7e` and `actor://a/~` denote the same instance.
///
/// Serialized as its URI string; ordered by it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorAddress {
    // `value` first: the derived ordering compares canonical strings.
    value: String,
    segments: Vec<String>,
}

impl ActorAddress {
    /// URI scheme prefix of every actor address.
    pub const SCHEME: &'static str = "actor://";

    /// The HTTP path segment that names an instance's mailbox, as in `/actors/{path}/messages`.
    pub const MAILBOX_SEGMENT: &'static str = "messages";

    /// [`Self::MAILBOX_SEGMENT`] with its first character percent-encoded: an actor segment, never the mailbox.
    pub const ESCAPED_MAILBOX_SEGMENT: &'static str = "%6Dessages";

    /// Parses a URI string into its canonical address.
    pub fn parse(uri: &str) -> Result<Self, AddressError> {
        let rest = uri
            .strip_prefix(Self::SCHEME)
            .ok_or_else(|| AddressError::new(format!("Actor URI must start with '{}': '{uri}'", Self::SCHEME)))?;
        if rest.is_empty() {
            return Err(AddressError::new(format!("Actor URI must have at least one segment: '{uri}'")));
        }
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(AddressError::new(format!("Actor URI segments must be non-empty: '{uri}'")));
        }
        let segments = parts.into_iter().map(percent::decode).collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from_segments_unchecked(segments))
    }

    /// Builds an address from raw (decoded) segment values; each value is percent-encoded as needed.
    pub fn of<I, S>(segments: I) -> Result<Self, AddressError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segments: Vec<String> = segments.into_iter().map(Into::into).collect();
        if segments.is_empty() {
            return Err(AddressError::new("An actor address needs at least one segment"));
        }
        if segments.iter().any(String::is_empty) {
            return Err(AddressError::new(format!("Actor address segments must be non-empty: {segments:?}")));
        }
        Ok(Self::from_segments_unchecked(segments))
    }

    fn from_segments_unchecked(segments: Vec<String>) -> Self {
        let encoded: Vec<String> = segments.iter().map(|s| percent::encode(s)).collect();
        Self { value: format!("{}{}", Self::SCHEME, encoded.join("/")), segments }
    }

    /// The canonical URI string, e.g. `actor://sync/acme`.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Decoded segment values, in order.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// True when this address's URI string starts with `prefix`; namespaces are plain string prefixes.
    pub fn has_prefix(&self, prefix: &str) -> bool {
        self.value.starts_with(prefix)
    }

    /// The HTTP path form of this address, e.g. `/actors/sync/acme` for `actor://sync/acme`.
    ///
    /// A trailing path segment `messages` addresses the mailbox endpoint (`/actors/{path}/messages`), so a last segment
    /// that is literally `messages` is written in its equivalent escaped form `%6Dessages`. Segments `.` and `..`
    /// would be removed by path normalisation in clients and proxies (RFC 3986 dot segments), so they are written as
    /// `%2E` and `%2E%2E`. Every escaped form decodes to the same address.
    pub fn to_http_path(&self) -> String {
        let mut encoded: Vec<String> = self
            .segments
            .iter()
            .map(|s| match s.as_str() {
                "." => "%2E".to_owned(),
                ".." => "%2E%2E".to_owned(),
                other => percent::encode(other),
            })
            .collect();
        if let Some(last) = encoded.last_mut()
            && last == Self::MAILBOX_SEGMENT
        {
            *last = Self::ESCAPED_MAILBOX_SEGMENT.to_owned();
        }
        format!("/actors/{}", encoded.join("/"))
    }

    /// Parses the path below `/actors/` (e.g. `sync/acme`, still percent-encoded) into an address.
    pub fn from_http_path(path: &str) -> Result<Self, AddressError> {
        Self::parse(&format!("{}{}", Self::SCHEME, path.trim_matches('/')))
    }
}

impl fmt::Display for ActorAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl fmt::Debug for ActorAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ActorAddress({})", self.value)
    }
}

impl FromStr for ActorAddress {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl AsRef<str> for ActorAddress {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl Serialize for ActorAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for ActorAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for ActorAddress {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ActorAddress".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "format": "uri", "pattern": "^actor://" })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonically() {
        let a = ActorAddress::parse("actor://a/%7e").unwrap();
        let b = ActorAddress::parse("actor://a/~").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "actor://a/~");
        assert_eq!(ActorAddress::of(["sync", "a b"]).unwrap().as_str(), "actor://sync/a%20b");
    }

    #[test]
    fn rejects_malformed() {
        for uri in ["sync/acme", "actor://", "actor://a//b", "actor://a/", "actor://a b"] {
            assert!(ActorAddress::parse(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn http_paths() {
        let a = ActorAddress::of(["a", "messages"]).unwrap();
        assert_eq!(a.to_http_path(), "/actors/a/%6Dessages");
        assert_eq!(ActorAddress::from_http_path("a/%6Dessages").unwrap(), a);
        let dots = ActorAddress::of([".", ".."]).unwrap();
        assert_eq!(dots.to_http_path(), "/actors/%2E/%2E%2E");
        assert_eq!(ActorAddress::from_http_path("%2E/%2E%2E").unwrap(), dots);
    }

    #[test]
    fn serde_as_string() {
        let a = ActorAddress::parse("actor://sync/acme").unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"actor://sync/acme\"");
        assert_eq!(serde_json::from_str::<ActorAddress>("\"actor://sync/acme\"").unwrap(), a);
    }
}
