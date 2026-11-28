use std::borrow::Cow;
use std::fmt;

use cicada::ActorAddress;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{Error, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::ReplyTo;

/// Where a target replies to a flow or an actor. A port can use `reply: ReplyRoute<T>` for both kinds of caller.
///
/// The flow form has `run` and `request`, the same fields as [`ReplyTo`]. The actor form has `actor` (an address)
/// and an optional `correlation` string. An object with `actor` cannot also have `run` or `request`.
#[derive(Serialize)]
#[serde(untagged, bound = "")]
pub enum ReplyRoute<T> {
    /// The run and request that are waiting for `T`.
    Flow(ReplyTo<T>),
    /// An actor that accepts `T` as a message of its own family.
    Actor {
        /// The caller's address.
        #[serde(rename = "actor")]
        to: ActorAddress,
        /// The correlation id carried back to the caller, when supplied.
        #[serde(skip_serializing_if = "Option::is_none")]
        correlation: Option<String>,
    },
}

impl<T> ReplyRoute<T> {
    /// Builds an actor route that takes its correlation id from the replying turn.
    pub fn actor(to: ActorAddress) -> Self {
        Self::Actor { to, correlation: None }
    }

    /// Sets the actor route's correlation id so it travels with the route into a later turn.
    /// A flow route keeps the correlation behavior of [`ReplyTo::send`].
    pub fn correlation(mut self, id: impl Into<String>) -> Self {
        if let Self::Actor { correlation, .. } = &mut self {
            *correlation = Some(id.into());
        }
        self
    }
}

impl<T> From<ReplyTo<T>> for ReplyRoute<T> {
    fn from(route: ReplyTo<T>) -> Self {
        Self::Flow(route)
    }
}

impl<T> Clone for ReplyRoute<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Flow(route) => Self::Flow(route.clone()),
            Self::Actor { to, correlation } => Self::Actor { to: to.clone(), correlation: correlation.clone() },
        }
    }
}

impl<T> fmt::Debug for ReplyRoute<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flow(route) => f.debug_tuple("Flow").field(route).finish(),
            Self::Actor { to, correlation } => {
                f.debug_struct("Actor").field("to", to).field("correlation", correlation).finish()
            }
        }
    }
}

impl<T> PartialEq for ReplyRoute<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Flow(a), Self::Flow(b)) => a == b,
            (Self::Actor { to: a, correlation: ac }, Self::Actor { to: b, correlation: bc }) => a == b && ac == bc,
            _ => false,
        }
    }
}

impl<T> Eq for ReplyRoute<T> {}

impl<'de, T> Deserialize<'de> for ReplyRoute<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RouteVisitor<T>(std::marker::PhantomData<fn() -> T>);

        impl<'de, T> Visitor<'de> for RouteVisitor<T> {
            type Value = ReplyRoute<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a reply route object")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let (mut actor, mut run, mut request) = (None, None, None);
                let mut correlation: Option<Value> = None;
                let mut duplicate_correlation = false;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "actor" => read_field(&mut actor, &mut map, "actor")?,
                        "run" => read_field(&mut run, &mut map, "run")?,
                        "request" => read_field(&mut request, &mut map, "request")?,
                        "correlation" => {
                            duplicate_correlation |= correlation.is_some();
                            correlation = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                if let Some(to) = actor {
                    if run.is_some() || request.is_some() {
                        return Err(M::Error::custom("`actor` cannot be combined with `run` or `request`"));
                    }
                    if duplicate_correlation {
                        return Err(M::Error::duplicate_field("correlation"));
                    }
                    let correlation = match correlation {
                        Some(Value::String(id)) => Some(id),
                        Some(_) => return Err(M::Error::custom("`correlation` must be a string")),
                        None => None,
                    };
                    Ok(ReplyRoute::Actor { to, correlation })
                } else {
                    let run = run.ok_or_else(|| M::Error::missing_field("run"))?;
                    let request = request.ok_or_else(|| M::Error::missing_field("request"))?;
                    Ok(ReplyRoute::Flow(ReplyTo::new(run, request)))
                }
            }
        }

        deserializer.deserialize_map(RouteVisitor(std::marker::PhantomData))
    }
}

fn read_field<'de, T: Deserialize<'de>, M: MapAccess<'de>>(
    field: &mut Option<T>,
    map: &mut M,
    name: &'static str,
) -> Result<(), M::Error> {
    if field.is_some() {
        return Err(M::Error::duplicate_field(name));
    }
    *field = Some(map.next_value()?);
    Ok(())
}

impl<T> JsonSchema for ReplyRoute<T> {
    fn schema_name() -> Cow<'static, str> {
        "ReplyRoute".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::ReplyRoute").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "oneOf": [
                generator.subschema_for::<ReplyTo<T>>(),
                {
                    "type": "object",
                    "properties": {
                        "actor": generator.subschema_for::<ActorAddress>(),
                        "correlation": {"type": "string"}
                    },
                    "required": ["actor"]
                }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Opaque;

    #[test]
    fn a_saved_reply_to_round_trips_as_a_flow_route() {
        let wire = json!({"run": "build/b1", "request": "m1"});
        let reply_to: ReplyTo<Opaque> = serde_json::from_value(wire.clone()).unwrap();
        let saved = serde_json::to_string(&reply_to).unwrap();
        let route = ReplyRoute::from(reply_to);
        assert_eq!(serde_json::from_str::<ReplyRoute<Opaque>>(&saved).unwrap(), route);
        assert_eq!(route.clone(), route);
        assert_eq!(serde_json::to_value(&route).unwrap(), wire);
        let with_extra = json!({"run": "build/b1", "request": "m1", "other": true, "correlation": 7});
        assert_eq!(serde_json::from_value::<ReplyRoute<Opaque>>(with_extra).unwrap(), route);
    }

    #[test]
    fn an_actor_route_round_trips_with_or_without_correlation() {
        let route = ReplyRoute::<Opaque>::actor(ActorAddress::parse("repos/a").unwrap());
        let plain = json!({"actor": "repos/a"});
        assert_eq!(serde_json::to_value(&route).unwrap(), plain);
        assert_eq!(serde_json::from_value::<ReplyRoute<Opaque>>(plain).unwrap(), route);

        let with_id = route.correlation("c1");
        let correlated = json!({"actor": "repos/a", "correlation": "c1"});
        assert_eq!(with_id.clone(), with_id);
        assert_eq!(serde_json::to_value(&with_id).unwrap(), correlated);
        assert_eq!(serde_json::from_value::<ReplyRoute<Opaque>>(correlated).unwrap(), with_id);
        let with_extra = json!({"actor": "repos/a", "correlation": "c1", "other": true});
        assert_eq!(serde_json::from_value::<ReplyRoute<Opaque>>(with_extra).unwrap(), with_id);
    }

    #[test]
    fn mixed_or_incomplete_routes_are_rejected() {
        for (wire, field) in [
            (json!({"actor": "repos/a", "run": "build/b1", "request": "m1"}), "actor"),
            (json!({"actor": "repos/a", "run": "build/b1"}), "run"),
            (json!({"actor": "repos/a", "request": "m1"}), "request"),
            (json!({"run": "build/b1"}), "request"),
            (json!({"request": "m1"}), "run"),
            (json!({}), "run"),
            (json!({"actor": "repos/a", "correlation": null}), "correlation"),
        ] {
            let error = serde_json::from_value::<ReplyRoute<Opaque>>(wire).unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }
        assert!(serde_json::from_str::<ReplyRoute<Opaque>>(r#"["build/b1", "m1"]"#).is_err());
        assert!(
            serde_json::from_value::<ReplyRoute<Opaque>>(json!({"actor": null, "run": "build/b1", "request": "m1"}))
                .is_err()
        );
    }

    #[test]
    fn the_schema_is_one_of_two_routes_regardless_of_reply_type() {
        let schema = serde_json::to_value(schemars::schema_for!(ReplyRoute<Opaque>)).unwrap();
        assert_eq!(schema, serde_json::to_value(schemars::schema_for!(ReplyRoute<u32>)).unwrap());
        assert_eq!(schema["title"], "ReplyRoute");
        assert_eq!(schema["oneOf"][0]["$ref"], "#/$defs/ReplyTo");
        assert_eq!(schema["oneOf"][1]["required"], json!(["actor"]));
        assert_eq!(schema["oneOf"][1]["properties"]["actor"]["type"], "string");
        assert_eq!(schema["oneOf"][1]["properties"]["correlation"]["type"], "string");
    }
}
