//! Serde bridge between a ref struct and its template parameters.
//!
//! A ref is a plain `#[derive(Serialize, Deserialize)]` struct (or unit struct) whose fields are exactly the template's
//! parameters. Each field is a scalar written as text: strings, booleans, integers, floats, chars, unit enum variants
//! and newtypes wrapping one of those. [`to_params`] writes a ref as `name -> text`; [`from_params`] parses the text
//! back, field type by field type, the way `serde_urlencoded` reads a form.

use std::collections::BTreeMap;
use std::fmt::{self, Display};

use serde::de::{self, DeserializeOwned, IntoDeserializer, Visitor};
use serde::ser::{self, Impossible, Serialize};

/// Why a ref could not be written as, or read from, template parameters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParamsError(String);

impl ser::Error for ParamsError {
    fn custom<T: Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

impl de::Error for ParamsError {
    fn custom<T: Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

/// Writes `value` (a struct of scalar fields, or a unit struct) as parameter texts by field name.
pub fn to_params<T: Serialize + ?Sized>(value: &T) -> Result<BTreeMap<String, String>, ParamsError> {
    value.serialize(RefSerializer)
}

/// Reads a ref of type `T` from parameter texts by field name.
pub fn from_params<T: DeserializeOwned>(params: &BTreeMap<String, String>) -> Result<T, ParamsError> {
    T::deserialize(RefDeserializer { params })
}

/// The field names of the struct `T`, as serde's derived `Deserialize` declares them; empty for a unit struct.
pub(crate) fn field_names<T: DeserializeOwned>() -> Result<Vec<&'static str>, String> {
    struct Probe<'a>(&'a mut Option<Vec<&'static str>>);

    impl<'de> de::Deserializer<'de> for Probe<'_> {
        type Error = ParamsError;

        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
            Err(ParamsError("not a struct".into()))
        }

        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _: &'static str,
            fields: &'static [&'static str],
            _: V,
        ) -> Result<V::Value, Self::Error> {
            *self.0 = Some(fields.to_vec());
            Err(ParamsError("probed".into()))
        }

        fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, _: V) -> Result<V::Value, Self::Error> {
            *self.0 = Some(Vec::new());
            Err(ParamsError("probed".into()))
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit
            newtype_struct seq tuple tuple_struct map enum identifier ignored_any
        }
    }

    let mut fields = None;
    let _ = T::deserialize(Probe(&mut fields));
    fields.ok_or_else(|| "a ref must be a struct with named fields or a unit struct".to_owned())
}

struct RefSerializer;

macro_rules! unsupported_ref {
    ($($method:ident($($arg:ty),*)),* $(,)?) => {
        $(fn $method(self, $(_: $arg),*) -> Result<Self::Ok, Self::Error> {
            Err(ParamsError("a ref must be a struct with named fields or a unit struct".into()))
        })*
    };
}

impl ser::Serializer for RefSerializer {
    type Ok = BTreeMap<String, String>;
    type Error = ParamsError;
    type SerializeSeq = Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = FieldCollector;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    unsupported_ref! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32), serialize_i64(i64),
        serialize_u8(u8), serialize_u16(u16), serialize_u32(u32), serialize_u64(u64), serialize_f32(f32),
        serialize_f64(f64), serialize_char(char), serialize_str(&str), serialize_bytes(&[u8]), serialize_none(),
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<Self::Ok, Self::Error> {
        Err(ParamsError("a ref must be a struct with named fields or a unit struct".into()))
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(BTreeMap::new())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(BTreeMap::new())
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(ParamsError("a ref must be a struct, not an enum".into()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(ParamsError("a ref must be a struct, not an enum".into()))
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(ParamsError("a ref must be a struct with named fields".into()))
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(ParamsError("a ref must be a struct with named fields".into()))
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(ParamsError("a ref must be a struct with named fields, not a tuple struct".into()))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(ParamsError("a ref must be a struct, not an enum".into()))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(ParamsError("a ref must be a struct with named fields, not a map".into()))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(FieldCollector(BTreeMap::new()))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(ParamsError("a ref must be a struct, not an enum".into()))
    }
}

struct FieldCollector(BTreeMap<String, String>);

impl ser::SerializeStruct for FieldCollector {
    type Ok = BTreeMap<String, String>;
    type Error = ParamsError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
        let text = value
            .serialize(ScalarSerializer)
            .map_err(|e| ParamsError(format!("Parameter '{key}': {}", e.0)))?;
        self.0.insert(key.to_owned(), text);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.0)
    }
}

/// Writes one scalar field as text.
struct ScalarSerializer;

macro_rules! scalar_display {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(fn $method(self, v: $ty) -> Result<Self::Ok, Self::Error> {
            Ok(v.to_string())
        })*
    };
}

impl ser::Serializer for ScalarSerializer {
    type Ok = String;
    type Error = ParamsError;
    type SerializeSeq = Impossible<String, ParamsError>;
    type SerializeTuple = Impossible<String, ParamsError>;
    type SerializeTupleStruct = Impossible<String, ParamsError>;
    type SerializeTupleVariant = Impossible<String, ParamsError>;
    type SerializeMap = Impossible<String, ParamsError>;
    type SerializeStruct = Impossible<String, ParamsError>;
    type SerializeStructVariant = Impossible<String, ParamsError>;

    scalar_display! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32), serialize_i64(i64),
        serialize_i128(i128), serialize_u8(u8), serialize_u16(u16), serialize_u32(u32), serialize_u64(u64),
        serialize_u128(u128), serialize_f32(f32), serialize_f64(f64), serialize_char(char), serialize_str(&str),
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<String, ParamsError> {
        Err(ParamsError("bytes are not a scalar".into()))
    }

    fn serialize_none(self) -> Result<String, ParamsError> {
        Err(ParamsError("ref properties cannot be optional".into()))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<String, ParamsError> {
        Err(ParamsError("ref properties cannot be optional".into()))
    }

    fn serialize_unit(self) -> Result<String, ParamsError> {
        Err(ParamsError("unit is not a scalar".into()))
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<String, ParamsError> {
        Err(ParamsError("unit is not a scalar".into()))
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<String, ParamsError> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<String, ParamsError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, ParamsError> {
        Err(ParamsError("only unit enum variants are scalars".into()))
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, ParamsError> {
        Err(ParamsError("a sequence is not a scalar".into()))
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, ParamsError> {
        Err(ParamsError("a tuple is not a scalar".into()))
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, ParamsError> {
        Err(ParamsError("a tuple struct is not a scalar".into()))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, ParamsError> {
        Err(ParamsError("only unit enum variants are scalars".into()))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, ParamsError> {
        Err(ParamsError("a map is not a scalar".into()))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, ParamsError> {
        Err(ParamsError("a struct is not a scalar".into()))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, ParamsError> {
        Err(ParamsError("only unit enum variants are scalars".into()))
    }
}

struct RefDeserializer<'a> {
    params: &'a BTreeMap<String, String>,
}

impl<'de> de::Deserializer<'de> for RefDeserializer<'_> {
    type Error = ParamsError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(ParamsAccess { iter: self.params.iter(), value: None })
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option
        seq tuple tuple_struct map struct enum identifier ignored_any
    }
}

struct ParamsAccess<'a, I: Iterator<Item = (&'a String, &'a String)>> {
    iter: I,
    value: Option<&'a String>,
}

impl<'de, 'a, I: Iterator<Item = (&'a String, &'a String)>> de::MapAccess<'de> for ParamsAccess<'a, I> {
    type Error = ParamsError;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                seed.deserialize(key.as_str().into_deserializer()).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        let value = self.value.take().ok_or_else(|| ParamsError("value without key".into()))?;
        seed.deserialize(ScalarDeserializer(value))
    }
}

/// Reads one scalar field from its text, parsing it as the field's type asks.
struct ScalarDeserializer<'a>(&'a str);

impl ScalarDeserializer<'_> {
    fn parse<T: std::str::FromStr>(&self, what: &str) -> Result<T, ParamsError> {
        self.0.parse().map_err(|_| ParamsError(format!("'{}' is not a valid {what}", self.0)))
    }
}

macro_rules! scalar_parse {
    ($($method:ident => $visit:ident: $ty:ty, $what:literal);* $(;)?) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            visitor.$visit(self.parse::<$ty>($what)?)
        })*
    };
}

impl<'de> de::Deserializer<'de> for ScalarDeserializer<'_> {
    type Error = ParamsError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_str(self.0)
    }

    scalar_parse! {
        deserialize_bool => visit_bool: bool, "boolean";
        deserialize_i8 => visit_i8: i8, "integer";
        deserialize_i16 => visit_i16: i16, "integer";
        deserialize_i32 => visit_i32: i32, "integer";
        deserialize_i64 => visit_i64: i64, "integer";
        deserialize_i128 => visit_i128: i128, "integer";
        deserialize_u8 => visit_u8: u8, "integer";
        deserialize_u16 => visit_u16: u16, "integer";
        deserialize_u32 => visit_u32: u32, "integer";
        deserialize_u64 => visit_u64: u64, "integer";
        deserialize_u128 => visit_u128: u128, "integer";
        deserialize_f32 => visit_f32: f32, "number";
        deserialize_f64 => visit_f64: f64, "number";
        deserialize_char => visit_char: char, "character";
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(self.0.into_deserializer())
    }

    serde::forward_to_deserialize_any! {
        str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

impl fmt::Debug for ScalarDeserializer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScalarDeserializer({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sync {
        tenant: String,
        job: u32,
        on: bool,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Cleanup;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Region {
        Eu,
        Us,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Tenant(String);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Regional {
        region: Region,
        tenant: Tenant,
    }

    #[test]
    fn round_trips_structs() {
        let s = Sync { tenant: "acme".into(), job: 7, on: true };
        let params = to_params(&s).unwrap();
        assert_eq!(params["job"], "7");
        assert_eq!(params["on"], "true");
        assert_eq!(from_params::<Sync>(&params).unwrap(), s);
        assert!(to_params(&Cleanup).unwrap().is_empty());
        assert_eq!(from_params::<Cleanup>(&BTreeMap::new()).unwrap(), Cleanup);
        let r = Regional { region: Region::Eu, tenant: Tenant("acme".into()) };
        let params = to_params(&r).unwrap();
        assert_eq!(params["region"], "eu");
        assert_eq!(from_params::<Regional>(&params).unwrap(), r);
    }

    #[test]
    fn rejects_bad_values() {
        let mut params = BTreeMap::new();
        params.insert("tenant".to_owned(), "acme".to_owned());
        params.insert("job".to_owned(), "x".to_owned());
        params.insert("on".to_owned(), "true".to_owned());
        assert!(from_params::<Sync>(&params).is_err());
    }

    #[test]
    fn probes_field_names() {
        assert_eq!(field_names::<Sync>().unwrap(), vec!["tenant", "job", "on"]);
        assert!(field_names::<Cleanup>().unwrap().is_empty());
        assert!(field_names::<String>().is_err());
    }
}
