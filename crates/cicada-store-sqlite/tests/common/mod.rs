//! Shared helpers: a temporary database file and a raw connection to it, as another process would have.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cicada::store::Clock;
use cicada::time::DEFAULT_RETENTION;
use cicada_store_sqlite::{SqliteOptions, SqliteStore};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection};
use tempfile::TempDir;

/// A temporary directory holding `actors.db`; removed when dropped.
pub struct TempDb {
    _dir: TempDir,
    pub file: PathBuf,
}

impl TempDb {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("cicada-sqlite").tempdir().expect("a temp dir");
        let file = dir.path().join("actors.db");
        Self { _dir: dir, file }
    }

    pub async fn open(&self, clock: Arc<dyn Clock>) -> SqliteStore {
        open_with(&self.file, clock, Duration::from_secs(5)).await
    }

    /// A raw connection, as another process would have.
    pub async fn raw(&self) -> SqliteConnection {
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&self.file).create_if_missing(true))
            .await
            .expect("a raw connection")
    }

    /// Runs `sql` (one or more statements) on a raw connection.
    pub async fn exec(&self, sql: &str) {
        let mut c = self.raw().await;
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned())).execute(&mut c).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
        c.close().await.expect("closed");
    }

    /// The text of a one-value query.
    pub async fn text(&self, sql: &str) -> Option<String> {
        let mut c = self.raw().await;
        let row = sqlx::query(AssertSqlSafe(sql.to_owned())).fetch_optional(&mut c).await.expect("query");
        let value = row.map(|r| {
            r.try_get::<String, _>(0)
                .or_else(|_| r.try_get::<i64, _>(0).map(|i| i.to_string()))
                .expect("text or integer")
        });
        c.close().await.expect("closed");
        value
    }

    pub async fn pragma(&self, name: &str) -> String {
        self.text(&format!("PRAGMA {name}")).await.expect("a pragma value")
    }

    pub async fn column_names(&self, sql: &str) -> Vec<String> {
        let mut c = self.raw().await;
        let rows = sqlx::query(AssertSqlSafe(sql.to_owned())).fetch_all(&mut c).await.expect("query");
        c.close().await.expect("closed");
        rows.iter().map(|r| r.get::<String, _>(0)).collect()
    }

    pub async fn columns(&self, table: &str) -> Vec<String> {
        self.column_names(&format!("SELECT name FROM pragma_table_info('{table}')")).await
    }
}

pub async fn open_with(file: &Path, clock: Arc<dyn Clock>, busy_timeout: Duration) -> SqliteStore {
    SqliteStore::open(file, SqliteOptions { clock, retention: DEFAULT_RETENTION, busy_timeout })
        .await
        .expect("the store opens")
}
