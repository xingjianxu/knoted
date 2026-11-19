//! Maintenance admission. PostgreSQL queues a conflicting exclusive waiter
//! ahead of later shared requests. Business SQL uses a different connection.
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;

const SHARED_LOCK: &str =
    "SELECT pg_try_advisory_xact_lock_shared(1263423300, current_schema()::regnamespace::oid::int)";

/// Dropping an unfinished guard conservatively preserves its marker. Only the
/// owner that has observed its operation finish may retire it.
#[derive(Debug)]
pub struct WriteGuard {
    tx: Transaction<'static, Postgres>,
    id: uuid::Uuid,
}

impl WriteGuard {
    pub async fn finish(mut self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM maintenance_operations WHERE id=$1")
            .bind(self.id)
            .execute(&mut *self.tx)
            .await?;
        self.tx.commit().await
    }
}

pub async fn try_write(
    mut tx: Transaction<'static, Postgres>,
    business: &PgPool,
) -> Result<Option<WriteGuard>, sqlx::Error> {
    let admitted: bool = sqlx::query_scalar(SHARED_LOCK).fetch_one(&mut *tx).await?;
    if !admitted {
        return Ok(None);
    }
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await?;
    let mut registration = business.begin().await?;
    // If the first backend disappeared while opening this connection, the
    // second shared lock still prevents an insertion under a granted backup.
    if !sqlx::query_scalar::<_, bool>(SHARED_LOCK)
        .fetch_one(&mut *registration)
        .await?
    {
        return Ok(None);
    }
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO maintenance_operations (id,backend_pid) VALUES ($1,$2)")
        .bind(id)
        .bind(pid)
        .execute(&mut *registration)
        .await?;
    registration.commit().await?;
    sqlx::query("SELECT 1").execute(&mut *tx).await?;
    Ok(Some(WriteGuard { tx, id }))
}

/// Owns the original exclusive transaction. A health check never reconnects.
#[derive(Debug)]
pub struct ExclusiveGuard(Transaction<'static, Postgres>);

impl ExclusiveGuard {
    pub async fn check(&mut self) -> Result<(), sqlx::Error> {
        let unresolved: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT FROM maintenance_operations)")
                .fetch_one(&mut *self.0)
                .await?;
        if unresolved {
            return Err(sqlx::Error::Protocol(
                "maintenance operation completion is unproven".into(),
            ));
        }
        Ok(())
    }
}

/// Acquire on a dedicated pool before opening the separate snapshot transaction.
pub async fn exclusive(pool: &PgPool, timeout: Duration) -> Result<ExclusiveGuard, sqlx::Error> {
    tokio::time::timeout(timeout, async {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('lock_timeout', $1, true)")
            .bind(format!(
                "{}ms",
                timeout.as_millis().clamp(1, i32::MAX as u128)
            ))
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(1263423300, current_schema()::regnamespace::oid::int)",
        )
        .execute(&mut *tx)
        .await?;
        let mut guard = ExclusiveGuard(tx);
        guard.check().await?;
        Ok(guard)
    })
    .await
    .map_err(|_| sqlx::Error::PoolTimedOut)?
}
