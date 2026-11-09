use std::collections::BTreeMap;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::params::{field_names, from_params, to_params};
use super::{ActorAddress, AddressError, TypeName};
use crate::message::MessageFamily;

/// A typed actor address: a struct whose fields are the key of an actor, bound to the actor's type name and to the
/// message family it accepts.
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
///     const TYPE: &'static str = "sync";
///     type Message = SyncMsg;
/// }
///
/// let address = Sync { tenant: "acme".into() }.address().unwrap();
/// assert_eq!(address.as_str(), "sync/acme");
/// ```
///
/// Supported field types are strings, booleans, integers, floats, chars, unit enum variants and newtypes wrapping one
/// of those; see [`super::to_params`]. Several fields form the key in declared order (`b1:7`). A unit struct is the ref
/// of a singleton (`cleanup/_`). The binding is checked when a node that hosts the type starts.
///
/// The compiler checks every typed send against [`Self::Message`]: `turn.send(&Sync { .. }, msg)` only accepts a
/// message of the sync family.
pub trait ActorRef: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The type name, e.g. `sync`; see [`TypeName`].
    const TYPE: &'static str;

    /// The message family this actor accepts.
    type Message: MessageFamily;

    /// The address of this ref, e.g. `sync/acme`.
    fn address(&self) -> Result<ActorAddress, AddressError> {
        RefBinding::<Self>::new()?.to_address(self)
    }
}

/// Two-way binding between a ref type `R` and its addresses: builds the address of a ref and decodes a ref from an
/// address.
pub struct RefBinding<R> {
    type_name: TypeName,
    fields: Vec<&'static str>,
    _ref: PhantomData<fn() -> R>,
}

impl<R> Clone for RefBinding<R> {
    fn clone(&self) -> Self {
        Self { type_name: self.type_name.clone(), fields: self.fields.clone(), _ref: PhantomData }
    }
}

impl<R: ActorRef> RefBinding<R> {
    /// The binding of `R`; fails when its type name is malformed or `R` is not a struct with named fields or a unit
    /// struct.
    pub fn new() -> Result<Self, AddressError> {
        let type_name = TypeName::parse(R::TYPE)?;
        let name = std::any::type_name::<R>();
        let fields = field_names::<R>().map_err(|e| AddressError::new(format!("{name}: {e}")))?;
        Ok(Self { type_name, fields, _ref: PhantomData })
    }

    /// The type name.
    pub fn type_name(&self) -> &TypeName {
        &self.type_name
    }

    /// The fields that form the key, in order.
    pub fn key_fields(&self) -> &[&'static str] {
        &self.fields
    }

    /// Builds the address of `r`.
    pub fn to_address(&self, r: &R) -> Result<ActorAddress, AddressError> {
        let params = to_params(r).map_err(|e| AddressError::new(e.to_string()))?;
        let mut values = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            values.push(params.get(*field).ok_or_else(|| AddressError::new(format!("The ref has no field {field}")))?);
        }
        ActorAddress::of(&self.type_name, values)
    }

    /// Decodes a ref from an address; fails when the address is of another type, or its key does not fit the ref.
    pub fn from_address(&self, address: &ActorAddress) -> Result<R, AddressError> {
        let name = std::any::type_name::<R>();
        if address.type_name() != &self.type_name {
            return Err(AddressError::new(format!("{address} is not of type {}", self.type_name)));
        }
        let params: BTreeMap<String, String> = if self.fields.is_empty() {
            if address.key() != ActorAddress::SINGLETON_KEY {
                return Err(AddressError::new(format!(
                    "{address}: {name} is a singleton, its key is {}",
                    ActorAddress::SINGLETON_KEY
                )));
            }
            BTreeMap::new()
        } else {
            let values = address.key_values();
            if values.len() != self.fields.len() {
                return Err(AddressError::new(format!(
                    "{address}: the key of {name} has {} values: {:?}",
                    self.fields.len(),
                    self.fields
                )));
            }
            self.fields.iter().map(|f| (*f).to_owned()).zip(values).collect()
        };
        from_params(&params).map_err(|e| AddressError::new(format!("Cannot decode {name} from {params:?}: {e}")))
    }
}
