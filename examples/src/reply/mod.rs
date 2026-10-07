//! Example 4 — request and reply.
//!
//! Messaging is one-way; a reply is an ordinary send to `message.from`, carrying the request's `correlation_id`. The
//! responder ([`Pricing`]) does not know its requesters' types: it replies to a raw address with
//! [`Turn::send_to`](cicada::Turn::send_to), so the reply's type name is checked against the TypeRegistry when it is
//! sent. Any actor whose message family contains messages named `Quote` and `QuoteRejected` with these fields can ask
//! for prices; here that is the [`Cart`].

mod cart;
mod pricing;

pub use cart::*;
pub use pricing::*;
