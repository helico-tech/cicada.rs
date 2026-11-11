//! Mapping of sqlx errors to [`StoreError`].

use crate::store::StoreError;

/// The SQLSTATE of a database error.
pub(crate) fn sql_state(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
        _ => None,
    }
}

/// True for an abort PostgreSQL asks to retry: serialization failure (`40001`) or deadlock (`40P01`).
pub(crate) fn is_transient(e: &sqlx::Error) -> bool {
    matches!(sql_state(e).as_deref(), Some("40001" | "40P01"))
}

/// Maps a sqlx error: values PostgreSQL refuses (SQLSTATE classes `22` data exception, e.g. a NUL character or
/// `\u0000` in a JSON string, and `54` program limit exceeded, e.g. an id too long for its index) are
/// [`StoreError::Invalid`]: they are caused by the request, and retrying cannot help; so are values sqlx cannot encode.
/// Values that cannot be decoded are
/// [`StoreError::Corrupt`]. Everything else (I/O, pool, other database errors) is [`StoreError::Unavailable`].
pub(crate) fn map_error(e: sqlx::Error) -> StoreError {
    if let Some(state) = sql_state(&e)
        && (state.starts_with("22") || state.starts_with("54"))
    {
        let message = match &e {
            sqlx::Error::Database(db) => db.message().to_owned(),
            other => other.to_string(),
        };
        return StoreError::Invalid(format!("PostgreSQL refused the data: {message}"));
    }
    match e {
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::TypeNotFound { .. } => StoreError::Corrupt(e.to_string()),
        sqlx::Error::Encode(_) => StoreError::Invalid(format!("PostgreSQL refused the data: {e}")),
        other => StoreError::Unavailable(Box::new(other)),
    }
}
