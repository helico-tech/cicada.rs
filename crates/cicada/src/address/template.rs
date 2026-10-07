use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{ActorAddress, AddressError, percent};

/// One segment of a [`UriTemplate`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment {
    /// Matches exactly one decoded value.
    Literal(String),
    /// Matches any single segment and binds it to the name.
    Param(String),
}

/// The URI template owned by an actor type, e.g. `actor://sync/{tenant}`.
///
/// Each segment is either a literal (matched exactly, after percent-decoding) or a `{param}` that matches any one
/// segment. Parameter names are identifiers and unique within a template. A template also identifies its actor type
/// in the store; its canonical string is [`Self::as_str`].
///
/// Serialized as its template string.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UriTemplate {
    value: String,
    segments: Vec<SegmentKey>,
}

/// A segment in a form that can be ordered (the derived order of `UriTemplate` uses `value` first anyway).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SegmentKey {
    Literal(String),
    Param(String),
}

impl UriTemplate {
    /// Parses a template string.
    pub fn parse(template: &str) -> Result<Self, AddressError> {
        let rest = template.strip_prefix(ActorAddress::SCHEME).ok_or_else(|| {
            AddressError::new(format!("Template must start with '{}': '{template}'", ActorAddress::SCHEME))
        })?;
        if rest.is_empty() {
            return Err(AddressError::new(format!("Template must have at least one segment: '{template}'")));
        }
        let mut segments = Vec::new();
        for raw in rest.split('/') {
            if raw.is_empty() {
                return Err(AddressError::new(format!("Template segments must be non-empty: '{template}'")));
            }
            if let Some(name) = raw.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
                if !is_param_name(name) {
                    return Err(AddressError::new(format!("Invalid parameter name '{name}' in '{template}'")));
                }
                segments.push(SegmentKey::Param(name.to_owned()));
            } else {
                if raw.contains('{') || raw.contains('}') {
                    return Err(AddressError::new(format!("Malformed parameter segment '{raw}' in '{template}'")));
                }
                segments.push(SegmentKey::Literal(percent::decode(raw)?));
            }
        }
        let mut names: Vec<&str> = segments
            .iter()
            .filter_map(|s| match s {
                SegmentKey::Param(n) => Some(n.as_str()),
                SegmentKey::Literal(_) => None,
            })
            .collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        if names.len() != count {
            return Err(AddressError::new(format!("Duplicate parameter name in '{template}'")));
        }
        let value = format!(
            "{}{}",
            ActorAddress::SCHEME,
            segments
                .iter()
                .map(|s| match s {
                    SegmentKey::Literal(v) => percent::encode(v),
                    SegmentKey::Param(n) => format!("{{{n}}}"),
                })
                .collect::<Vec<_>>()
                .join("/")
        );
        Ok(Self { value, segments })
    }

    /// The canonical template string, e.g. `actor://sync/{tenant}`.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// The template's segments, in order.
    pub fn segments(&self) -> Vec<Segment> {
        self.segments
            .iter()
            .map(|s| match s {
                SegmentKey::Literal(v) => Segment::Literal(v.clone()),
                SegmentKey::Param(n) => Segment::Param(n.clone()),
            })
            .collect()
    }

    /// Names of the parameters, in segment order.
    pub fn param_names(&self) -> Vec<&str> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                SegmentKey::Param(n) => Some(n.as_str()),
                SegmentKey::Literal(_) => None,
            })
            .collect()
    }

    /// The template's shape: its literals with every parameter replaced by one placeholder. Two templates with the
    /// same shape match exactly the same addresses. Stores keep it in a unique column; the wider rule is
    /// [`Self::overlaps`].
    pub fn shape(&self) -> String {
        self.segments
            .iter()
            .map(|s| match s {
                SegmentKey::Literal(v) => percent::encode(v),
                SegmentKey::Param(_) => "{}".to_owned(),
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// An address that both this template and `other` match, or `None` when no address matches both.
    ///
    /// Two templates share an address exactly when they have the same number of segments and, at every position, at
    /// least one of them has a parameter or both have the same literal. The address returned takes the literal where
    /// there is one and the parameter's name otherwise: `actor://jobs/{id}` and `actor://jobs/cleanup` share
    /// `actor://jobs/cleanup`.
    pub fn common_address(&self, other: &UriTemplate) -> Option<ActorAddress> {
        if self.segments.len() != other.segments.len() {
            return None;
        }
        let mut common = Vec::with_capacity(self.segments.len());
        for (a, b) in self.segments.iter().zip(&other.segments) {
            let value = match (a, b) {
                (SegmentKey::Literal(x), SegmentKey::Literal(y)) => {
                    if x != y {
                        return None;
                    }
                    x.clone()
                }
                (SegmentKey::Literal(x), _) | (_, SegmentKey::Literal(x)) => x.clone(),
                (SegmentKey::Param(n), SegmentKey::Param(_)) => n.clone(),
            };
            common.push(value);
        }
        ActorAddress::of(common).ok()
    }

    /// True when some address matches both this template and `other`. An equal template overlaps itself.
    pub fn overlaps(&self, other: &UriTemplate) -> bool {
        self.common_address(other).is_some()
    }

    /// Matches `address` against this template: the decoded parameter values by name, or `None` when it does not
    /// match.
    pub fn matches(&self, address: &ActorAddress) -> Option<BTreeMap<String, String>> {
        if address.segments().len() != self.segments.len() {
            return None;
        }
        let mut params = BTreeMap::new();
        for (segment, actual) in self.segments.iter().zip(address.segments()) {
            match segment {
                SegmentKey::Literal(v) => {
                    if v != actual {
                        return None;
                    }
                }
                SegmentKey::Param(n) => {
                    params.insert(n.clone(), actual.clone());
                }
            }
        }
        Some(params)
    }

    /// Builds the address for the given parameter values; fails when a parameter is missing, unknown or empty.
    pub fn expand(&self, params: &BTreeMap<String, String>) -> Result<ActorAddress, AddressError> {
        let names = self.param_names();
        if let Some(unknown) = params.keys().find(|k| !names.contains(&k.as_str())) {
            return Err(AddressError::new(format!("Unknown parameter '{unknown}' for template {}", self.value)));
        }
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in &self.segments {
            match segment {
                SegmentKey::Literal(v) => segments.push(v.clone()),
                SegmentKey::Param(n) => match params.get(n) {
                    Some(v) => segments.push(v.clone()),
                    None => {
                        return Err(AddressError::new(format!("Missing parameter '{n}' for {}", self.value)));
                    }
                },
            }
        }
        ActorAddress::of(segments)
    }
}

fn is_param_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl fmt::Display for UriTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl fmt::Debug for UriTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UriTemplate({})", self.value)
    }
}

impl FromStr for UriTemplate {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for UriTemplate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for UriTemplate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> UriTemplate {
        UriTemplate::parse(s).unwrap()
    }

    #[test]
    fn parses_and_matches() {
        let template = t("actor://sync/{tenant}");
        assert_eq!(template.param_names(), vec!["tenant"]);
        let params = template.matches(&ActorAddress::parse("actor://sync/acme").unwrap()).unwrap();
        assert_eq!(params["tenant"], "acme");
        assert!(template.matches(&ActorAddress::parse("actor://sync/a/b").unwrap()).is_none());
        assert_eq!(template.expand(&params).unwrap().as_str(), "actor://sync/acme");
        assert_eq!(template.shape(), "sync/{}");
    }

    #[test]
    fn overlaps() {
        assert!(t("actor://jobs/{id}").overlaps(&t("actor://jobs/cleanup")));
        assert_eq!(
            t("actor://jobs/{id}").common_address(&t("actor://jobs/cleanup")).unwrap().as_str(),
            "actor://jobs/cleanup"
        );
        assert!(!t("actor://jobs/{id}").overlaps(&t("actor://jobs/{id}/steps/{n}")));
        assert!(!t("actor://e2e/counter/{id}").overlaps(&t("actor://e2e/slow/{id}")));
    }

    #[test]
    fn rejects_malformed() {
        for s in ["sync/{a}", "actor://", "actor://a/{1x}", "actor://a/{x}/{x}", "actor://a/x{y}", "actor://a//b"] {
            assert!(UriTemplate::parse(s).is_err(), "{s}");
        }
    }
}
