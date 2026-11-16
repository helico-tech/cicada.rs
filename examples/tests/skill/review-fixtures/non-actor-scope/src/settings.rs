// The admin settings editor: one HTTP handler, one transaction. Cicada actors elsewhere in the service read these rows.

use sqlx::PgPool;

pub struct Change {
    pub key: String,
    pub value: String,
    pub expected_version: i64,
}

/// Applies all changes or none; a stale `expected_version` refuses the whole save.
pub async fn save(pool: &PgPool, admin: &str, changes: &[Change]) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    for change in changes {
        let updated = sqlx::query(
            "UPDATE settings SET value = $1, version = version + 1, changed_by = $2 WHERE key = $3 AND version = $4",
        )
        .bind(&change.value)
        .bind(admin)
        .bind(&change.key)
        .bind(change.expected_version)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Ok(false); // dropping tx rolls back
        }
    }
    tx.commit().await?;
    Ok(true)
}
