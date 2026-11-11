use std::any::type_name;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{ActorAddress, AddressError, TypeName, reader};
use crate::message::MessageFamily;

/// A typed actor address: a type that serde writes as the one name of an actor (ADR 0021), bound to the actor's type
/// name and to the message family it accepts.
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
/// A ref is one of three shapes, as serde sees it: a struct of which serde writes one field, whose value is a string (a
/// `String`, a newtype around one, a `Uuid`; a field with `#[serde(skip)]` does not count); a type that serde writes as
/// a string itself (a newtype around a `String`, or `#[serde(into = "String", try_from = "String")]`); or a unit struct
/// or a struct that declares no field to serde, the ref of a singleton (`cleanup/_`); a ref that declares a field and
/// writes none has no address. A number, a boolean, a character and an enum variant are not a string. The one field
/// cannot have a serde alias, and a ref cannot use a reader that takes more than one form (`deserialize_any`, `from` an
/// untagged enum): the check counts what the reader declares. A ref with two parts joins them itself, with a separator
/// the parts cannot hold. A node that hosts the type checks the shape when it starts; [`Self::address`] fails for a ref
/// that does not write one string.
///
/// The compiler checks every typed send against [`Self::Message`]: `turn.send(&Sync { .. }, msg)` only accepts a
/// message of the sync family.
pub trait ActorRef: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The type name, e.g. `sync`; see [`TypeName`].
    const TYPE: &'static str;

    /// The message family this actor accepts.
    type Message: MessageFamily;

    /// The address of this ref, e.g. `sync/acme`; fails when the type name is malformed or serde does not write this
    /// ref as one non-empty string or a unit.
    fn address(&self) -> Result<ActorAddress, AddressError> {
        let ty = TypeName::parse(Self::TYPE)?;
        let invalid =
            |e: &dyn std::fmt::Display| AddressError::new(format!("{} has no address: {e}", type_name::<Self>()));
        let name = reader::name_of(self).map_err(|e| invalid(&e))?;
        ActorAddress::of(&ty, &name).map_err(|e| invalid(&e))
    }
}

/// Two-way binding between a ref type `R` and its addresses: builds the address of a ref and decodes a ref from an
/// address.
pub struct RefBinding<R> {
    type_name: TypeName,
    _ref: PhantomData<fn() -> R>,
}

impl<R> Clone for RefBinding<R> {
    fn clone(&self) -> Self {
        Self { type_name: self.type_name.clone(), _ref: PhantomData }
    }
}

impl<R: ActorRef> RefBinding<R> {
    /// The binding of `R`; fails when its type name is malformed, or when what the `Deserialize` of `R` asks for is
    /// not one string or a unit.
    pub fn new() -> Result<Self, AddressError> {
        let ty = TypeName::parse(R::TYPE)?;
        reader::check::<R>().map_err(|e| AddressError::new(format!("{}: {e}", type_name::<R>())))?;
        Ok(Self { type_name: ty, _ref: PhantomData })
    }

    /// The type name.
    pub fn type_name(&self) -> &TypeName {
        &self.type_name
    }

    /// Builds the address of `r`: [`ActorRef::address`].
    pub fn to_address(&self, r: &R) -> Result<ActorAddress, AddressError> {
        r.address()
    }

    /// Decodes a ref from an address; fails when the address is of another type, or the ref does not accept its name
    /// (a singleton accepts only `_`).
    pub fn from_address(&self, address: &ActorAddress) -> Result<R, AddressError> {
        if address.type_name() != &self.type_name {
            return Err(AddressError::new(format!("{address} is not of type {}", self.type_name)));
        }
        reader::from_name(&address.name()).map_err(AddressError::new)
    }
}
