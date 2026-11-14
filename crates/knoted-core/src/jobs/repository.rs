//! SQL for the job queue.
//!
//! Every function is a single short transaction or a single statement: workers
//! and request handlers never hold a transaction across the actual work, so a
//! slow job cannot block the claim scan or an enqueue.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    EnqueueOutcome, Job, JobOutcome, JobSpec, JobState, LEASE_SECONDS, Lease, enqueued_topic,
};
use crate::db::DbError;

/// SQL predicate shared by the claim scan: a job is due when it is waiting for
/// its first or next attempt and its availability has passed, or when it is
/// running/cancelling but its lease expired (the owner is gone, so the attempt
/// has to be repeated).
const DUE_PREDICATE: &str = "((state IN ('queued', 'retry_wait') AND available_at <= now())
      OR (state IN ('running', 'cancel_requested') AND lease_expires_at < now()))";

/// Enqueues `spec` in the caller's transaction.
///
/// Both the job row and its outbox row are written here, so the event exists
/// exactly when the job exists.
///
/// When `(kind, idempotency_key)` already exists the existing job is returned
/// with [`EnqueueOutcome::created`] set to `false` and no second outbox row is
/// written: a retried enqueue is not a new piece of work. The existing job may
/// already have finished, so the caller must look at `created`/`job.state`
/// instead of assuming the work will run. Jobs without a key always insert.
///
/// `max_attempts` below 1 is rejected with [`DbError::InvalidMaxAttempts`]
/// rather than silently clamped, so a caller cannot believe a job will be tried
/// when it would have been tried zero times.
pub async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    spec: JobSpec,
) -> Result<EnqueueOutcome, DbError> {
    if spec.max_attempts < 1 {
        return Err(DbError::InvalidMaxAttempts(spec.max_attempts));
    }
    let inserted: Option<Job> = sqlx::query_as(
        "INSERT INTO jobs (kind, idempotency_key, payload, priority, max_attempts, available_at) \
         VALUES ($1, $2, $3, $4, $5, COALESCE($6, now())) \
         ON CONFLICT (kind, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING \
         RETURNING *",
    )
    .bind(&spec.kind)
    .bind(&spec.idempotency_key)
    .bind(&spec.payload)
    .bind(spec.priority)
    .bind(spec.max_attempts)
    .bind(spec.available_at)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(job) = inserted else {
        // The conflict target only matches rows carrying a key, so this branch
        // runs exactly when a job with this key already exists.
        let Some(key) = spec.idempotency_key.as_deref() else {
            return Err(DbError::UnkeyedIdempotentConflict);
        };
        let existing: Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind = $1 AND idempotency_key = $2")
                .bind(&spec.kind)
                .bind(key)
                .fetch_one(&mut **tx)
                .await?;
        return Ok(EnqueueOutcome {
            job: existing,
            created: false,
        });
    };

    sqlx::query("INSERT INTO outbox (job_id, topic, payload) VALUES ($1, $2, $3)")
        .bind(job.id)
        .bind(enqueued_topic(&job.kind))
        .bind(job.outbox_payload())
        .execute(&mut **tx)
        .await?;

    Ok(EnqueueOutcome { job, created: true })
}

/// Claims one due job for `worker`, or returns `None` when nothing is due.
///
/// The row is locked with `FOR UPDATE SKIP LOCKED` inside a short transaction,
/// so concurrent workers take different jobs instead of queueing behind one
/// row. An expired lease is up for grabs: the takeover writes a new token and
/// counts as another attempt, and any result the previous holder still produces
/// is rejected by [`heartbeat`] and [`finish_with_token`].
///
/// A takeover keeps `cancel_requested` as it is: taking the lease over repeats
/// the attempt, it does not undo the cancellation request. Every other due job
/// becomes `running`. The returned [`Lease::state`] is the state the job was in
/// when it was claimed, so the caller can see that it is confirming a
/// cancellation instead of running the work.
pub async fn claim(pool: &PgPool, worker: &str) -> Result<Option<Lease>, DbError> {
    let token = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let row: Option<(Uuid,)> = sqlx::query_as(&format!(
        "SELECT id FROM jobs WHERE {DUE_PREDICATE} \
         ORDER BY priority DESC, available_at ASC, created_at ASC \
         LIMIT 1 FOR UPDATE SKIP LOCKED"
    ))
    .fetch_optional(&mut *tx)
    .await?;

    let Some((job_id,)) = row else {
        tx.rollback().await?;
        return Ok(None);
    };

    let (attempt, expires_at, state): (i32, DateTime<Utc>, String) = sqlx::query_as(
        "UPDATE jobs SET state = CASE WHEN state = 'cancel_requested' \
                 THEN 'cancel_requested' ELSE 'running' END, \
             lease_token = $2, \
             lease_expires_at = now() + make_interval(secs => $3), \
             attempts = attempts + 1, locked_by = $4, updated_at = now() \
         WHERE id = $1 \
         RETURNING attempts, lease_expires_at, state",
    )
    .bind(job_id)
    .bind(token)
    .bind(LEASE_SECONDS)
    .bind(worker)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let state = JobState::parse(&state).ok_or(DbError::UnknownJobState(state))?;
    Ok(Some(Lease {
        job_id,
        token,
        expires_at,
        attempt,
        worker: worker.to_owned(),
        state,
    }))
}

/// Extends the lease of a job this worker still owns and reports the state it
/// was in while doing so.
///
/// `None` means the token no longer matches (another worker took the job over),
/// exactly like [`heartbeat`] returning `false`. `Some(CancelRequested)` tells
/// the worker to stop the execution and confirm the cancellation instead of
/// committing a result.
pub async fn heartbeat_state(pool: &PgPool, lease: &Lease) -> Result<Option<JobState>, DbError> {
    let row: Option<(String,)> = sqlx::query_as(
        "UPDATE jobs SET lease_expires_at = now() + make_interval(secs => $3), updated_at = now() \
         WHERE id = $1 AND lease_token = $2 AND state IN ('running', 'cancel_requested') \
         RETURNING state",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(LEASE_SECONDS)
    .fetch_optional(pool)
    .await?;

    let Some((state,)) = row else {
        return Ok(None);
    };
    JobState::parse(&state)
        .map(Some)
        .ok_or(DbError::UnknownJobState(state))
}

/// Extends the lease of a job this worker still owns.
///
/// Returns `false` when the token no longer matches, that is when another worker
/// has taken the job over. The caller must stop working in that case: nothing it
/// produces afterwards can be committed.
pub async fn heartbeat(pool: &PgPool, lease: &Lease) -> Result<bool, DbError> {
    Ok(heartbeat_state(pool, lease).await?.is_some())
}

/// Reads one job row, or `None` when it does not exist.
pub async fn load(pool: &PgPool, job_id: Uuid) -> Result<Option<Job>, DbError> {
    Ok(sqlx::query_as("SELECT * FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(pool)
        .await?)
}

/// Requests cancellation in one atomic statement.
///
/// A `queued` or `retry_wait` job has no lease and therefore goes straight to
/// `cancelled`; `cancel_requested` requires the lease that only a running job
/// holds, so a running job keeps it until the worker confirms. Returns `None`
/// when the job does not exist or is already `cancel_requested`/terminal: those
/// states are left untouched and the caller reads the row to report it.
pub async fn request_cancel(pool: &PgPool, job_id: Uuid) -> Result<Option<Job>, DbError> {
    Ok(sqlx::query_as(
        "UPDATE jobs SET state = CASE \
                 WHEN state IN ('queued', 'retry_wait') THEN 'cancelled' \
                 ELSE 'cancel_requested' END, \
             updated_at = now() \
         WHERE id = $1 AND state IN ('queued', 'retry_wait', 'running') \
         RETURNING *",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?)
}

/// Moves a still-owned attempt to `retry_wait` and sets its next availability.
///
/// The three lease columns are cleared in the same statement (the check
/// constraint requires it), `attempts` is left untouched because the retry
/// budget is consumed by claims, not by scheduling, and `last_error` keeps the
/// reason for the next attempt. Returns `false` when the token no longer owns
/// the job; the database is untouched in that case.
pub async fn schedule_retry(
    pool: &PgPool,
    lease: &Lease,
    error: &str,
    available_at: DateTime<Utc>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET state = 'retry_wait', last_error = $3, available_at = $4, \
             lease_token = NULL, lease_expires_at = NULL, locked_by = NULL, updated_at = now() \
         WHERE id = $1 AND lease_token = $2 AND state IN ('running', 'cancel_requested')",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(error)
    .bind(available_at)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

/// Records that `job_id` was created by a manual retry of `retry_of`.
pub async fn link_retry(
    tx: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
    retry_of: Uuid,
) -> Result<Job, DbError> {
    Ok(sqlx::query_as(
        "UPDATE jobs SET retry_of = $2, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(job_id)
    .bind(retry_of)
    .fetch_one(&mut **tx)
    .await?)
}

/// Commits `outcome` if `lease` still owns the job, and releases the lease.
///
/// Returns `false` when the token no longer matches (or the job already left
/// `running`/`cancel_requested`), in which case the database is untouched. The
/// caller must not treat `false` as success: the job belongs to another attempt.
pub async fn finish_with_token(
    pool: &PgPool,
    lease: &Lease,
    outcome: JobOutcome,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET state = $3, last_error = $4, lease_token = NULL, \
             lease_expires_at = NULL, locked_by = NULL, updated_at = now() \
         WHERE id = $1 AND lease_token = $2 AND state IN ('running', 'cancel_requested')",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(outcome.state())
    .bind(outcome.last_error())
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}
