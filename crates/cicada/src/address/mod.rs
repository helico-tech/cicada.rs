//! Actor addresses: a type name plus a key, written `sync/acme`, and the typed refs that build them.

mod actor_ref;
mod params;
pub mod percent;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use actor_ref::{ActorRef, RefBinding};
pub use params::{from_params, to_params};

/// A malformed type name or actor address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AddressError(String);

impl AddressError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The name of an actor type, e.g. `sync` or `billing.v2.invoices`: it matches `[a-z][a-z0-9._-]*`, at most
/// [`Self::MAX_LENGTH`] characters. It is the type's identity in the store.
///
/// Serialized as its string.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeName(String);

impl TypeName {
    /// The longest type name.
    pub const MAX_LENGTH: usize = 100;

    /// Checks `name`.
    pub fn parse(name: &str) -> Result<Self, AddressError> {
        let valid = name.len() <= Self::MAX_LENGTH
            && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(AddressError::new(format!(
                "Invalid type name '{name}': use [a-z][a-z0-9._-]*, at most {} characters",
                Self::MAX_LENGTH
            )))
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeName({})", self.0)
    }
}

impl FromStr for TypeName {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for TypeName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TypeName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// The address of one actor instance: its type name and its key, written `type/key` (`sync/acme`) and always held in
/// canonical form.
///
/// A key is one value (`acme`), or several joined with `:` in the order of the ref's fields (`b1:7`). A singleton has
/// the key [`Self::SINGLETON_KEY`]. Each value is non-empty and uses `A-Z a-z 0-9 . _ ~ -`; any other character is
/// percent-encoded (see [`percent`]), so a key never holds a `/`, and a `:` is always a separator. Two addresses are
/// equal exactly when their canonical strings are equal, so `a/%7e` and `a/~` denote the same instance. The key `.` or
/// `..` is refused, because URL parsing would remove it.
///
/// Serialized as its text form; ordered by it. The text form is also the address's path below `/actors/` over HTTP.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorAddress {
    // `value` first: the derived ordering compares canonical strings.
    value: String,
    type_name: TypeName,
}

impl ActorAddress {
    /// The key of a singleton: the instance of a type whose ref has no fields.
    pub const SINGLETON_KEY: &'static str = "_";

    /// Parses `type/key` into its canonical address.
    pub fn parse(text: &str) -> Result<Self, AddressError> {
        let (type_name, key) = text
            .split_once('/')
            .ok_or_else(|| AddressError::new(format!("An actor address is type/key, was '{text}'")))?;
        let type_name = TypeName::parse(type_name)?;
        let values = key.split(':').map(percent::decode).collect::<Result<Vec<_>, _>>()?;
        Self::of(&type_name, values).map_err(|e| AddressError::new(format!("Invalid address '{text}': {e}")))
    }

    /// Builds the address of `type_name` from raw (decoded) key values, each percent-encoded as needed. No values is
    /// the singleton.
    pub fn of<I, S>(type_name: &TypeName, values: I) -> Result<Self, AddressError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut encoded = Vec::new();
        for value in values {
            let value = value.as_ref();
            if value.is_empty() {
                return Err(AddressError::new("Key values must be non-empty"));
            }
            encoded.push(percent::encode(value));
        }
        let key = if encoded.is_empty() { Self::SINGLETON_KEY.to_owned() } else { encoded.join(":") };
        if key == "." || key == ".." {
            return Err(AddressError::new(format!("The key must not be '{key}'")));
        }
        Ok(Self { value: format!("{type_name}/{key}"), type_name: type_name.clone() })
    }

    /// The canonical text form, e.g. `sync/acme`.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// The type name.
    pub fn type_name(&self) -> &TypeName {
        &self.type_name
    }

    /// The canonical key, e.g. `acme` or `b1:7`.
    pub fn key(&self) -> &str {
        &self.value[self.type_name.as_str().len() + 1..]
    }

    /// The decoded key values, in order.
    pub fn key_values(&self) -> Vec<String> {
        self.key().split(':').map(|v| percent::decode(v).expect("a canonical key decodes")).collect()
    }

    /// True when this address's text form starts with `prefix`: `sync/` is one type, `sync/ac` a key prefix.
    pub fn has_prefix(&self, prefix: &str) -> bool {
        self.value.starts_with(prefix)
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
        schemars::json_schema!({ "type": "string", "pattern": "^[a-z][a-z0-9._-]*/[^/]+$" })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(n: &str) -> TypeName {
        TypeName::parse(n).unwrap()
    }

    #[test]
    fn type_names() {
        for good in ["sync", "billing.v2.invoices", "a-b_c", "x1"] {
            assert_eq!(name(good).as_str(), good);
        }
        for bad in ["", "Sync", "1a", ".a", "a/b", "a b", &"a".repeat(101)] {
            assert!(TypeName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_to_canonical_form() {
        let a = ActorAddress::parse("sync/acme").unwrap();
        assert_eq!((a.type_name().as_str(), a.key(), a.as_str()), ("sync", "acme", "sync/acme"));
        assert_eq!(ActorAddress::parse("a/%7e").unwrap(), ActorAddress::parse("a/~").unwrap());
        assert_eq!(ActorAddress::parse("b/x:7").unwrap().key_values(), ["x", "7"]);
        for bad in ["sync", "sync/", "/x", "sync/a/b", "sync/a:", "sync/:a", "Sync/a", "a/b c", "a/.", "a/..", "a/%2E"]
        {
            assert!(ActorAddress::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn values_with_separators_are_encoded() {
        let a = ActorAddress::of(&name("files"), ["a/b c", "d:e"]).unwrap();
        assert_eq!(a.as_str(), "files/a%2Fb%20c:d%3Ae");
        assert_eq!(a.key_values(), ["a/b c", "d:e"]);
        assert_eq!(ActorAddress::parse(a.as_str()).unwrap(), a);
    }

    #[test]
    fn a_ref_without_fields_is_the_singleton() {
        let a = ActorAddress::of(&name("cleanup"), Vec::<String>::new()).unwrap();
        assert_eq!(a.as_str(), "cleanup/_");
        assert!(ActorAddress::of(&name("x"), [""]).is_err());
    }
}
