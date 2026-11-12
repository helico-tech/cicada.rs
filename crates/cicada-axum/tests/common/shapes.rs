//! Three types that refer to themselves with several schema steps between two of their levels, as the third round of
//! validation found them: with the first validator their check needed 86 to 103 KiB of stack a level in a test build;
//! the check of the door as it is needs about 2.7 KiB (decision 11); and a list that holds itself. Shared by the test
//! of the check function in `src/schema_check.rs` and the tests of the HTTP door in `tests/deep_values.rs`.
#![allow(dead_code)]

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// A type that refers to itself, with a value one level deep and a way to wrap a value in one more level.
pub trait Shape: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {
    /// Its type name in a test.
    const NAME: &'static str;
    /// The innermost value, one level deep: one that matches, or one with a mismatch.
    fn leaf(ok: bool) -> Value;
    /// `inner` one level deeper.
    fn wrap(inner: Value) -> Value;
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Map<_, _>>())
}

fn pick(ok: bool) -> Value {
    if ok { json!(1) } else { json!("x") }
}

/// A list that holds itself through an `Option`, with no step between two levels.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct List {
    n: i32,
    next: Option<Box<List>>,
}

impl Shape for List {
    const NAME: &'static str = "list";
    fn leaf(ok: bool) -> Value {
        object(vec![("n", pick(ok)), ("next", Value::Null)])
    }
    fn wrap(inner: Value) -> Value {
        object(vec![("n", json!(1)), ("next", inner)])
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C1(C2);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C2(C3);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C3(C4);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C4(C5);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C5(C6);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct C6(Chain);

/// Six newtypes between one level and the next.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Chain {
    n: i32,
    next: Option<Box<C1>>,
}

impl Shape for Chain {
    const NAME: &'static str = "chain";
    fn leaf(ok: bool) -> Value {
        object(vec![("n", pick(ok)), ("next", Value::Null)])
    }
    fn wrap(inner: Value) -> Value {
        object(vec![("n", json!(1)), ("next", inner)])
    }
}

/// An adjacently tagged enum, then an `Option`, then an untagged enum, then two newtypes, between two levels.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", content = "c")]
pub enum Stack {
    Leaf(i32),
    Wrap(Option<Either>),
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Either {
    Text(String),
    More(Box<S1>),
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct S1(S2);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct S2(Stack);

impl Shape for Stack {
    const NAME: &'static str = "stack";
    fn leaf(ok: bool) -> Value {
        object(vec![("t", json!("Leaf")), ("c", pick(ok))])
    }
    fn wrap(inner: Value) -> Value {
        object(vec![("t", json!("Wrap")), ("c", inner)])
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct W1(W2);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct W2(W3);
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct W3(Wide);

/// A tagged enum under an `Option` under three newtypes.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "k")]
pub enum Wide {
    Leaf { v: i32 },
    Node { next: Option<Box<W1>>, label: Option<String> },
}

impl Shape for Wide {
    const NAME: &'static str = "wide";
    fn leaf(ok: bool) -> Value {
        object(vec![("k", json!("Leaf")), ("v", pick(ok))])
    }
    fn wrap(inner: Value) -> Value {
        object(vec![("k", json!("Node")), ("next", inner), ("label", Value::Null)])
    }
}

/// A value of `T` of `levels` levels (each wrap adds one), with the mismatch, if any, in its innermost level. Built
/// without recursion.
pub fn build<T: Shape>(levels: usize, ok: bool) -> Value {
    let mut value = T::leaf(ok);
    for _ in 1..levels {
        value = T::wrap(value);
    }
    value
}

/// The message family of a test type of shape `T`: one message whose payload is `{"v": T}`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum PutMsg<T> {
    Put { v: T },
}

/// The payload of a `Put` of `levels` levels: `{"v": …}` around a value of one level less.
pub fn payload<T: Shape>(levels: usize, ok: bool) -> Value {
    object(vec![("v", build::<T>(levels - 1, ok))])
}
