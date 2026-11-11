//! The reader between a ref and its name (ADR 0021).
//!
//! A ref is written as one string or as a unit: a struct of which serde writes one field, whose value is a string; a
//! type that serde writes as a string itself (a newtype around a `String`, a type that names `String` as its serde
//! form); or a unit struct, the singleton with the name `_`. A field that serde skips does not count. A number, a
//! boolean, a character and an enum variant are not a string.
//!
//! [`name_of`] takes the name out of a ref through its `Serialize`, [`from_name`] puts it back through its
//! `Deserialize`, and [`check`] tells from what that `Deserialize` asks for whether a ref is one string, before any
//! value exists.

use std::fmt::{self, Display};

use serde::de::{self, DeserializeOwned, IntoDeserializer, Visitor};
use serde::ser::{self, Impossible, Serialize};

use super::ActorAddress;

/// The rule a ref breaks, at the start of every refusal.
const RULE: &str = "a ref is written as one string or as a unit";

/// The name of `r`: the string serde writes for it, or `_` for a unit struct.
pub(crate) fn name_of<R: Serialize + ?Sized>(r: &R) -> Result<String, String> {
    r.serialize(Writer { field: None }).map_err(|e| e.to_string())
}

/// The ref of type `R` that serde reads from `name`.
pub(crate) fn from_name<R: DeserializeOwned>(name: &str) -> Result<R, String> {
    R::deserialize(Reader { name: Some(name), field: None }).map_err(|e| e.to_string())
}

/// Checks, by what the `Deserialize` of `R` asks for, that `R` is read as one string or as a unit. It stops where it
/// would need a value, so a ref that refuses some names passes.
pub(crate) fn check<R: DeserializeOwned>() -> Result<(), String> {
    match R::deserialize(Reader { name: None, field: None }) {
        Ok(_) | Err(Error::Probed) => Ok(()),
        Err(Error::Refused(e)) => Err(e),
    }
}

#[derive(Debug)]
enum Error {
    /// The probe of [`check`] reached the string.
    Probed,
    /// Why the ref is not one string, or the ref's own refusal of a name.
    Refused(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Probed => f.write_str("no name to read"),
            Self::Refused(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Self::Refused(msg.to_string())
    }
}

impl de::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Self::Refused(msg.to_string())
    }
}

/// Writes a ref, or the value of its one field, as its name.
#[derive(Clone, Copy)]
struct Writer {
    /// The field being written; `None` for the ref itself.
    field: Option<&'static str>,
}

impl Writer {
    fn refuse<T>(self, what: &str) -> Result<T, Error> {
        Err(Error::Refused(match self.field {
            None => format!("{RULE}, but serde writes {what}"),
            Some(field) => format!("{RULE}, but serde writes {what} for its field {field}"),
        }))
    }

    fn unit(self) -> Result<String, Error> {
        match self.field {
            None => Ok(ActorAddress::SINGLETON_KEY.to_owned()),
            Some(_) => self.refuse("a unit"),
        }
    }
}

macro_rules! write_refused {
    ($($method:ident($($arg:ty),*) => $what:literal),* $(,)?) => {
        $(fn $method(self, $(_: $arg),*) -> Result<String, Error> {
            self.refuse($what)
        })*
    };
}

impl ser::Serializer for Writer {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = Impossible<String, Error>;
    type SerializeTuple = Impossible<String, Error>;
    type SerializeTupleStruct = Impossible<String, Error>;
    type SerializeTupleVariant = Impossible<String, Error>;
    type SerializeMap = Impossible<String, Error>;
    type SerializeStruct = OneName;
    type SerializeStructVariant = Impossible<String, Error>;

    write_refused! {
        serialize_bool(bool) => "a boolean", serialize_i8(i8) => "a number", serialize_i16(i16) => "a number",
        serialize_i32(i32) => "a number", serialize_i64(i64) => "a number", serialize_i128(i128) => "a number",
        serialize_u8(u8) => "a number", serialize_u16(u16) => "a number", serialize_u32(u32) => "a number",
        serialize_u64(u64) => "a number", serialize_u128(u128) => "a number", serialize_f32(f32) => "a number",
        serialize_f64(f64) => "a number", serialize_char(char) => "a character", serialize_bytes(&[u8]) => "bytes",
        serialize_none() => "an option", serialize_unit_variant(&'static str, u32, &'static str) => "an enum variant",
    }

    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(v.to_owned())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<String, Error> {
        self.refuse("an option")
    }

    fn serialize_unit(self) -> Result<String, Error> {
        self.unit()
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<String, Error> {
        self.unit()
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<String, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, Error> {
        self.refuse("an enum variant")
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        self.refuse("a sequence")
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        self.refuse("a tuple")
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Error> {
        self.refuse("a tuple struct")
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        self.refuse("an enum variant")
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        self.refuse("a map")
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        match self.field {
            None => Ok(OneName(None)),
            Some(_) => self.refuse("a struct"),
        }
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        self.refuse("an enum variant")
    }
}

/// The fields of a ref struct, of which serde writes one.
struct OneName(Option<String>);

impl ser::SerializeStruct for OneName {
    type Ok = String;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        if self.0.is_some() {
            return Err(Error::Refused(format!("{RULE}, but serde writes a second field {key}")));
        }
        self.0 = Some(value.serialize(Writer { field: Some(key) })?);
        Ok(())
    }

    fn end(self) -> Result<String, Error> {
        self.0.ok_or_else(|| Error::Refused(format!("{RULE}, but serde writes a struct without fields")))
    }
}

/// Reads a ref, or the value of its one field, from a name; without a name it is the probe of [`check`].
#[derive(Clone, Copy)]
struct Reader<'a> {
    name: Option<&'a str>,
    /// The field being read; `None` for the ref itself.
    field: Option<&'static str>,
}

impl Reader<'_> {
    fn refuse<T>(self, what: &str) -> Result<T, Error> {
        Err(Error::Refused(match self.field {
            None => format!("{RULE}, but it asks for {what}"),
            Some(field) => format!("{RULE}, but its field {field} asks for {what}"),
        }))
    }

    fn string<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.name {
            None => Err(Error::Probed),
            Some(name) => visitor.visit_str(name),
        }
    }

    fn unit<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match (self.field, self.name) {
            (Some(_), _) => self.refuse("a unit"),
            (None, None | Some(ActorAddress::SINGLETON_KEY)) => visitor.visit_unit(),
            (None, Some(name)) => Err(Error::Refused(format!(
                "the ref is a singleton, its key is {}, not {name}",
                ActorAddress::SINGLETON_KEY
            ))),
        }
    }
}

macro_rules! read_refused {
    ($($method:ident => $what:literal),* $(,)?) => {
        $(fn $method<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
            self.refuse($what)
        })*
    };
}

impl<'de> de::Deserializer<'de> for Reader<'_> {
    type Error = Error;

    read_refused! {
        deserialize_any => "any value", deserialize_bool => "a boolean", deserialize_i8 => "a number",
        deserialize_i16 => "a number", deserialize_i32 => "a number", deserialize_i64 => "a number",
        deserialize_i128 => "a number", deserialize_u8 => "a number", deserialize_u16 => "a number",
        deserialize_u32 => "a number", deserialize_u64 => "a number", deserialize_u128 => "a number",
        deserialize_f32 => "a number", deserialize_f64 => "a number", deserialize_char => "a character",
        deserialize_bytes => "bytes", deserialize_byte_buf => "bytes", deserialize_option => "an option",
        deserialize_seq => "a sequence", deserialize_map => "a map", deserialize_identifier => "an identifier",
        deserialize_ignored_any => "any value",
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.string(visitor)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.unit(visitor)
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Error> {
        self.unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, Error> {
        self.refuse("a tuple")
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _: &'static str, _: usize, _: V) -> Result<V::Value, Error> {
        self.refuse("a tuple struct")
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match (self.field, fields) {
            (Some(_), _) => self.refuse("a struct"),
            (None, &[field]) => {
                visitor.visit_map(OneField { field, value: Some(Reader { field: Some(field), ..self }) })
            }
            (None, []) => self.refuse("a struct without fields"),
            (None, fields) => self.refuse(&format!("the fields {}", fields.join(", "))),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Error> {
        self.refuse("an enum")
    }
}

/// The one entry of a ref struct: its field and the name.
struct OneField<'a> {
    field: &'static str,
    value: Option<Reader<'a>>,
}

impl<'de> de::MapAccess<'de> for OneField<'_> {
    type Error = Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        match self.value {
            Some(_) => seed.deserialize(IntoDeserializer::<Error>::into_deserializer(self.field)).map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        seed.deserialize(self.value.take().ok_or_else(|| Error::Refused("a value without a key".into()))?)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Tenant {
        tenant: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Cleanup;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Two {
        a: String,
        b: String,
    }

    #[test]
    fn a_ref_with_one_field_round_trips() {
        let r = Tenant { tenant: "acme".into() };
        assert_eq!(name_of(&r).unwrap(), "acme");
        assert_eq!(from_name::<Tenant>("acme").unwrap(), r);
        assert!(check::<Tenant>().is_ok());
    }

    #[test]
    fn a_unit_struct_is_the_singleton() {
        assert_eq!(name_of(&Cleanup).unwrap(), "_");
        assert_eq!(from_name::<Cleanup>("_").unwrap(), Cleanup);
        assert!(from_name::<Cleanup>("other").unwrap_err().contains("singleton"));
        assert!(check::<Cleanup>().is_ok());
    }

    #[test]
    fn a_ref_with_two_fields_is_refused_both_ways() {
        let written = name_of(&Two { a: "x".into(), b: "y".into() }).unwrap_err();
        assert!(written.contains("a second field b"), "{written}");
        let asked = check::<Two>().unwrap_err();
        assert!(asked.contains("the fields a, b"), "{asked}");
    }

    #[test]
    fn a_string_itself_is_one_name() {
        assert_eq!(name_of("acme").unwrap(), "acme");
        assert_eq!(from_name::<String>("acme").unwrap(), "acme");
        assert!(check::<String>().is_ok());
        assert!(check::<u32>().unwrap_err().contains("asks for a number"));
    }
}
