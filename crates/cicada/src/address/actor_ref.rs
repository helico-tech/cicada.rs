use std::collections::BTreeMap;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::params::{field_names, from_params, to_params};
use super::{ActorAddress, AddressError, UriTemplate};
use crate::message::MessageFamily;

/// A typed actor address: a struct whose fields are exactly the parameters of its URI template, bound to that
/// template and to the message family the actor accepts.
///
/// ```
/// use cicada::ActorRef;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// struct Sync {
///     tenant: String,
/// }
///
/// # #[derive(Serialize, Deserialize, schemars::JsonSchema)]
/// # #[serde(tag = "type")]
/// # enum SyncMsg { Run }
/// impl ActorRef for Sync {
///     const TEMPLATE: &'static str = "actor://sync/{tenant}";
///     type Message = SyncMsg;
/// }
///
/// let address = Sync { tenant: "acme".into() }.address().unwrap();
/// assert_eq!(address.as_str(), "actor://sync/acme");
/// ```
///
/// Supported field types are strings, booleans, integers, floats, chars, unit enum variants and newtypes wrapping one
/// of those; see [`super::to_params`]. A unit struct is the ref of a template without parameters (a singleton such as
/// `actor://maintenance/cleanup`). The binding is checked when a node that hosts the type starts.
///
/// The compiler checks every typed send against [`Self::Message`]: `turn.send(&Sync { .. }, msg)` only accepts a
/// message of the sync family.
pub trait ActorRef: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The URI template, e.g. `actor://sync/{tenant}`.
    const TEMPLATE: &'static str;

    /// The message family this actor accepts.
    type Message: MessageFamily;

    /// The address of this ref, built from its template, e.g. `actor://sync/acme`.
    fn address(&self) -> Result<ActorAddress, AddressError> {
        RefBinding::<Self>::new()?.to_address(self)
    }
}

/// Two-way binding between a ref type `R` and its [`UriTemplate`]: builds the URI from a ref and decodes a ref from
/// the template parameters of a URI.
pub struct RefBinding<R> {
    template: UriTemplate,
    _ref: PhantomData<fn() -> R>,
}

impl<R> Clone for RefBinding<R> {
    fn clone(&self) -> Self {
        Self { template: self.template.clone(), _ref: PhantomData }
    }
}

impl<R: ActorRef> RefBinding<R> {
    /// The binding of `R`; fails when its template is malformed.
    pub fn new() -> Result<Self, AddressError> {
        Ok(Self { template: UriTemplate::parse(R::TEMPLATE)?, _ref: PhantomData })
    }

    /// Everything wrong with the binding: a malformed template, or fields that are not exactly the template's
    /// parameters. Empty when it is valid.
    pub fn problems() -> Vec<String> {
        let template = match UriTemplate::parse(R::TEMPLATE) {
            Ok(t) => t,
            Err(e) => return vec![e.to_string()],
        };
        let name = std::any::type_name::<R>();
        match field_names::<R>() {
            Ok(fields) => {
                let mut fields: Vec<&str> = fields;
                let mut params = template.param_names();
                fields.sort_unstable();
                params.sort_unstable();
                if fields == params {
                    Vec::new()
                } else {
                    vec![format!("{name}: fields {fields:?} must equal the parameters {params:?} of {template}")]
                }
            }
            Err(e) => vec![format!("{name}: {e}")],
        }
    }

    /// The template.
    pub fn template(&self) -> &UriTemplate {
        &self.template
    }

    /// Builds the address of `r`.
    pub fn to_address(&self, r: &R) -> Result<ActorAddress, AddressError> {
        let params = to_params(r).map_err(|e| AddressError::new(e.to_string()))?;
        self.template.expand(&params)
    }

    /// Decodes a ref from template parameter values.
    pub fn from_params(&self, params: &BTreeMap<String, String>) -> Result<R, AddressError> {
        from_params(params).map_err(|e| {
            AddressError::new(format!("Cannot decode {} from {params:?}: {e}", std::any::type_name::<R>()))
        })
    }

    /// Decodes a ref from an address; fails when the address does not match the template or a parameter does not
    /// decode.
    pub fn from_address(&self, address: &ActorAddress) -> Result<R, AddressError> {
        let params = self
            .template
            .matches(address)
            .ok_or_else(|| AddressError::new(format!("{address} does not match {}", self.template)))?;
        self.from_params(&params)
    }
}
