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

/// Fixed operator-facing classifications. HTTP responses, addresses and
/// database connection details never form part of a backup diagnostic.
#[derive(Debug, thiserror::Error)]
pub enum QuiescenceError {
    #[error("maintenance database proof unavailable")]
    Database,
    #[error("job lease or cancellation still active")]
    Job,
    #[error("chat provider stop unconfirmed")]
    Chat,
    #[error("external submission receipt unconfirmed")]
    Submission,
    #[error("intelligence service exit unconfirmed")]
    Python,
    #[error("search queue completion unconfirmed")]
    Search,
    #[error("external destination differs from the recorded submission target")]
    Destination,
    #[error("maintenance quiescence check timed out")]
    Timeout,
}

impl From<sqlx::Error> for QuiescenceError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

/// Call while holding the exclusive maintenance guard. This read-only probe
/// never resends, cancels or clears a remote submission to manufacture proof.
pub async fn verify_quiescence(ctx: &crate::context::AppContext) -> Result<(), QuiescenceError> {
    tokio::time::timeout(Duration::from_secs(60), verify_external_state(ctx))
        .await
        .map_err(|_| QuiescenceError::Timeout)?
}

async fn verify_external_state(ctx: &crate::context::AppContext) -> Result<(), QuiescenceError> {
    use crate::intelligence::{ExecutionRequest, HealthState, IntelligenceClient};
    use crate::search::meili::MeiliClient;
    use std::collections::BTreeSet;

    let pool = ctx.pool();
    let (busy_job, busy_chat, pending_meili): (bool,bool,bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT FROM jobs WHERE lease_expires_at > clock_timestamp() OR state='cancel_requested'),
                EXISTS (SELECT FROM generation_runs WHERE upstream_requested AND NOT upstream_stopped
                    AND (admitted OR state NOT IN ('completed','failed','cancelled','interrupted'))),
                EXISTS (SELECT FROM maintenance_external_submissions WHERE service='meili')"
    ).fetch_one(pool).await?;
    if busy_job {
        return Err(QuiescenceError::Job);
    }
    if busy_chat {
        return Err(QuiescenceError::Chat);
    }
    if pending_meili {
        return Err(QuiescenceError::Submission);
    }

    let mut python_required = ctx.files().enabled || ctx.vectors_enabled();
    let mut indexes = BTreeSet::new();
    let targets: Vec<(String, String, String)> =
        sqlx::query_as("SELECT service,origin,index_uid FROM maintenance_external_targets")
            .fetch_all(pool)
            .await?;
    for (service, origin, index) in targets {
        match service.as_str() {
            "python" if origin == ctx.intelligence().base_url.trim_end_matches('/') => {
                python_required = true
            }
            "meili" if origin == ctx.search().base_url.trim_end_matches('/') => {
                indexes.insert(index);
            }
            _ => return Err(QuiescenceError::Destination),
        }
    }
    let requests: Vec<sqlx::types::Json<ExecutionRequest>> = sqlx::query_scalar(
        "SELECT DISTINCT execution_request FROM maintenance_external_submissions WHERE service='python'"
    ).fetch_all(pool).await?;
    if python_required || !requests.is_empty() {
        let client = IntelligenceClient::from_context(ctx).map_err(|_| QuiescenceError::Python)?;
        for request in requests {
            // GET cannot enqueue work. A matching durable terminal record also
            // makes a still-delayed idempotent POST harmless.
            let record = client
                .get_execution(request.execution_id)
                .await
                .map_err(|_| QuiescenceError::Submission)?;
            record
                .verify_resource_exit(&request)
                .map_err(|_| QuiescenceError::Submission)?;
        }
        let health = client.health().await.map_err(|_| QuiescenceError::Python)?;
        if health.status != HealthState::Ok || health.active_executions != 0 {
            return Err(QuiescenceError::Python);
        }
    }
    if ctx.search().enabled {
        indexes.insert(ctx.search().index_uid.clone());
        indexes.insert(ctx.search().rag_chunk_index_uid.clone());
    }
    let historic: Vec<String> = sqlx::query_scalar(
        "SELECT index_uid FROM fulltext_publications WHERE meili_task_uid IS NOT NULL OR published_at IS NOT NULL
         UNION SELECT index_uid FROM rag_chunk_publications WHERE meili_task_uid IS NOT NULL OR last_confirmed_meili_task_uid IS NOT NULL OR published_at IS NOT NULL"
    ).fetch_all(pool).await?;
    indexes.extend(historic);
    if !indexes.is_empty() {
        let client = MeiliClient::from_context(ctx).map_err(|_| QuiescenceError::Search)?;
        for uid in indexes {
            if !client
                .queue_is_empty(&uid)
                .await
                .map_err(|_| QuiescenceError::Search)?
            {
                return Err(QuiescenceError::Search);
            }
        }
    }
    Ok(())
}
