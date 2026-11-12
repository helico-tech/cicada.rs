/// A value that an actor of this crate cannot keep: a message that is no message of a family, a state that does not
/// encode or is `null`, or a ref without an address. It prints why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EncodeError(pub(crate) String);

impl From<cicada::message::MessageError> for EncodeError {
    fn from(e: cicada::message::MessageError) -> Self {
        Self(e.to_string())
    }
}
