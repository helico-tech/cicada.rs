//! Mapping between rows and the store model: instants as INTEGER microseconds, JSON as TEXT.

use cicada::address::{ActorAddress, UriTemplate};
use cicada::retention::Retention;
use cicada::retry::RetryPolicy;
use cicada::store::support;
use cicada::store::{ActorRecord, Message, MessageId, OutcomeRecord, OutcomeStatus, StoreError, StoreResult, TypeDef};
use cicada::time::{self, Timestamp};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{Decode, Row, Sqlite, Type};

/// The value of column `name`; a value of the wrong type is corrupt data.
pub(crate) fn col<'r, T: Decode<'r, Sqlite> + Type<Sqlite>>(row: &'r SqliteRow, name: &str) -> StoreResult<T> {
    row.try_get(name).map_err(|e| StoreError::Corrupt(format!("column {name}: {e}")))
}

pub(crate) fn micros(instant: Timestamp) -> i64 {
    time::to_micros(instant)
}

pub(crate) fn instant(micros: i64) -> Timestamp {
    time::from_micros(micros)
}

/// Compact JSON text, as Kotlin's `Json.encodeToString(JsonElement)` writes it.
pub(crate) fn json(value: &Value) -> String {
    value.to_string()
}

pub(crate) fn parse_json(text: &str, what: &str) -> StoreResult<Value> {
    serde_json::from_str(text).map_err(|e| StoreError::Corrupt(format!("{what} is not JSON: {e}")))
}

pub(crate) fn address(uri: &str) -> StoreResult<ActorAddress> {
    ActorAddress::parse(uri).map_err(|e| StoreError::Corrupt(format!("stored address '{uri}': {e}")))
}

pub(crate) fn template(text: &str) -> StoreResult<UriTemplate> {
    UriTemplate::parse(text).map_err(|e| StoreError::Corrupt(format!("stored template '{text}': {e}")))
}

pub(crate) fn message_id(id: String) -> StoreResult<MessageId> {
    MessageId::new(id).map_err(StoreError::Corrupt)
}

fn count(value: i64, what: &str) -> StoreResult<u32> {
    u32::try_from(value).map_err(|_| StoreError::Corrupt(format!("{what} out of range: {value}")))
}

pub(crate) fn type_def(row: &SqliteRow) -> StoreResult<TypeDef> {
    Ok(TypeDef {
        template: template(&col::<String>(row, "template")?)?,
        contract: parse_json(&col::<String>(row, "contract")?, "contract")?,
        contract_hash: col(row, "contract_hash")?,
    })
}

pub(crate) fn actor_record(row: &SqliteRow) -> StoreResult<ActorRecord> {
    Ok(ActorRecord {
        address: address(&col::<String>(row, "uri")?)?,
        template: template(&col::<String>(row, "type")?)?,
        state: col::<Option<String>>(row, "state")?.map(|s| parse_json(&s, "state")).transpose()?,
        version: col(row, "version")?,
        created_at: instant(col(row, "created_at")?),
    })
}

/// A `messages` row. A retry or retention policy this version cannot decode reads as no policy, so that it cannot
/// break claims (Kotlin review L1).
pub(crate) fn message(row: &SqliteRow) -> StoreResult<Message> {
    let id: String = col(row, "id")?;
    let source = format!("message {id}");
    Ok(Message {
        to: address(&col::<String>(row, "actor_uri")?)?,
        from: col::<Option<String>>(row, "sender")?.map(|s| address(&s)).transpose()?,
        message_type: col(row, "type")?,
        payload: parse_json(&col::<String>(row, "payload")?, "payload")?,
        deliver_at: instant(col(row, "deliver_at")?),
        correlation_id: col(row, "correlation_id")?,
        headers: support::headers_from_json(&parse_json(&col::<String>(row, "headers")?, "headers")?)?,
        seq: col(row, "seq")?,
        attempt: count(col(row, "attempt")?, "attempt")?,
        failures: count(col(row, "failures")?, "failures")?,
        last_error: col(row, "last_error")?,
        retry: col::<Option<String>>(row, "retry_policy")?.and_then(|t| RetryPolicy::from_stored_text(&t, &source)),
        caused_by: col::<Option<String>>(row, "caused_by")?.map(message_id).transpose()?,
        retention: col::<Option<String>>(row, "retention_policy")?
            .and_then(|t| Retention::from_stored_text(&t, &source)),
        id: message_id(id)?,
    })
}

pub(crate) fn outcome(row: &SqliteRow) -> StoreResult<OutcomeRecord> {
    let status: String = col(row, "status")?;
    Ok(OutcomeRecord {
        message_id: message_id(col(row, "message_id")?)?,
        actor: address(&col::<String>(row, "actor_uri")?)?,
        status: OutcomeStatus::from_wire_name(&status)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown outcome status '{status}'")))?,
        error: col(row, "error")?,
        finished_at: instant(col(row, "finished_at")?),
        envelope: support::envelope_from_json(&parse_json(&col::<String>(row, "envelope")?, "envelope")?)?,
    })
}
