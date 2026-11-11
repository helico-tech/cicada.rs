//! A connection with explicitly controlled transactions.
//!
//! Transactions are plain `BEGIN IMMEDIATE` / `BEGIN DEFERRED` … `COMMIT` statements, not sqlx's `Transaction`, so the
//! write lock is taken at `BEGIN` and a transaction never fails half-way on a lock. A `BEGIN` that fails with
//! `SQLITE_BUSY` (another process held the write lock longer than the busy timeout) is retried a few times; when it
//! still fails, nothing has started. Any error rolls the transaction back, so the connection never stays inside one.
//! A future dropped mid-transaction (a cancelled caller) leaves the connection marked as possibly open, and the next
//! `BEGIN` rolls that transaction back first.

use crate::store::{StoreError, StoreResult};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

/// How a transaction starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Takes the write lock at once (`BEGIN IMMEDIATE`), so the transaction never fails half-way on a lock.
    Write,
    /// A read snapshot (`BEGIN DEFERRED`); never waits for writers in WAL mode.
    Read,
}

impl Mode {
    fn begin(self) -> &'static str {
        match self {
            Mode::Write => "BEGIN IMMEDIATE",
            Mode::Read => "BEGIN DEFERRED",
        }
    }
}

/// Attempts to start a write transaction when another connection holds the write lock past the busy timeout.
const BEGIN_ATTEMPTS: u32 = 3;

/// A database or I/O error: the store cannot serve the request right now.
pub(crate) fn db_error(e: sqlx::Error) -> StoreError {
    StoreError::Unavailable(Box::new(e))
}

/// `SQLITE_BUSY` or one of its extended codes.
fn is_busy(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| code & 0xFF == 5)
}

/// A connection and whether a transaction may be open on it.
pub(crate) struct Conn {
    inner: SqliteConnection,
    open: bool,
}

impl Conn {
    pub(crate) async fn connect(options: &SqliteConnectOptions) -> StoreResult<Self> {
        let inner = SqliteConnection::connect_with(options).await.map_err(db_error)?;
        Ok(Self { inner, open: false })
    }

    /// The connection, for statements inside the current transaction.
    pub(crate) fn raw(&mut self) -> &mut SqliteConnection {
        &mut self.inner
    }

    /// Starts a transaction, first rolling back one that a cancelled caller left open.
    pub(crate) async fn begin(&mut self, mode: Mode) -> Result<(), sqlx::Error> {
        if self.open {
            self.rollback().await;
        }
        // Marked before BEGIN is sent: if this future is dropped while BEGIN runs, the next use rolls back.
        self.open = true;
        let mut attempt = 1;
        loop {
            match sqlx::raw_sql(mode.begin()).execute(&mut self.inner).await {
                Ok(_) => return Ok(()),
                Err(e) if is_busy(&e) && attempt < BEGIN_ATTEMPTS => attempt += 1,
                Err(e) => {
                    self.rollback().await;
                    return Err(e);
                }
            }
        }
    }

    /// Commits `result`'s transaction when it is `Ok`, rolls it back otherwise; a failed commit rolls back too.
    pub(crate) async fn end<T>(&mut self, result: StoreResult<T>) -> StoreResult<T> {
        match result {
            Ok(value) => match sqlx::raw_sql("COMMIT").execute(&mut self.inner).await {
                Ok(_) => {
                    self.open = false;
                    Ok(value)
                }
                Err(e) => {
                    self.rollback().await;
                    Err(db_error(e))
                }
            },
            Err(e) => {
                self.rollback().await;
                Err(e)
            }
        }
    }

    /// Rolls back, ignoring the error SQLite gives when it already rolled back by itself (no transaction is active).
    pub(crate) async fn rollback(&mut self) {
        if let Err(e) = sqlx::raw_sql("ROLLBACK").execute(&mut self.inner).await {
            tracing::trace!("ROLLBACK failed (usually: no transaction is active): {e}");
        }
        self.open = false;
    }

    pub(crate) async fn close(self) {
        if let Err(e) = self.inner.close().await {
            tracing::debug!("Closing a SQLite connection failed: {e}");
        }
    }
}
