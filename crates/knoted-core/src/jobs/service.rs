//! Job state machine: automatic retries, cancellation and manual retries.
//!
//! The primitives in [`super`] move one row between two states with a lease
//! token; this module decides *which* transition is allowed and computes the
//! backoff delay. Keeping the decisions here means a worker cannot invent its
//! own retry policy and every caller sees the same 5/30/120 second schedule.
//!
//! All time comes from [`AppContext::now`], never from `Utc::now()`, so tests
//! advance a [`crate::context::FixedClock`] instead of sleeping.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Job, JobOutcome, JobSpec, JobState, Lease, enqueue, repository};
use crate::context::AppContext;
use crate::db::DbError;
use crate::files::scanner;

/// Automatic retry delays, indexed by the attempt that failed.
///
/// The first failed attempt waits 5 seconds, the second 30 and the third 120.
/// After that the budget is exhausted and the job becomes `failed`.
pub const RETRY_BACKOFF_SECONDS: [i64; 3] = [5, 30, 120];

/// Failures of the job state machine.
#[derive(Debug, thiserror::Error)]
pub enum DomainJobError {
    #[error("job {0} not found")]
    NotFound(Uuid),
    #[error("lease for job {job_id} is stale or no longer held")]
    LeaseLost { job_id: Uuid },
    #[error("job {job_id} is `{state}` and cannot be retried")]
    NotRetryable { job_id: Uuid, state: String },
    #[error("job {job_id} is `{state}` and has no cancellation to confirm")]
    CancelNotRequested { job_id: Uuid, state: String },
    #[error(
        "cancellation for job {job_id} could not be applied because the state kept changing \
         (last `{state}`)"
    )]
    CancelContended { job_id: Uuid, state: String },
    #[error("job {job_id} is already terminal ({state})")]
    CancelTerminal { job_id: Uuid, state: String },
    #[error("a file scan is already pending")]
    ScanPending,
    #[error(transparent)]
    Database(#[from] DbError),
}

/// What a worker observed while executing one attempt.
///
/// The distinction between temporary and permanent decides whether the attempt
/// budget is spent on a retry; it is not derived from the error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutcome {
    /// The work finished and its result may be committed.
    Succeeded,
    /// The attempt failed for a reason that may go away, such as a timeout.
    TemporaryFailure { message: String },
    /// The attempt failed for a reason a retry cannot fix.
    PermanentFailure { message: String },
    /// The handler reports that the execution ended as cancelled.
    Cancelled,
}

/// Input of [`finish_execution`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishExecution {
    /// Job the worker held the lease on.
    pub job_id: Uuid,
    /// Token the worker received from `claim`; a stale token is rejected.
    pub lease_token: Uuid,
    /// What the attempt produced.
    pub outcome: ExecutionOutcome,
    /// Whether the worker confirmed that the cancelled execution really exited.
    ///
    /// Only meaningful while the job is `cancel_requested`: `false` keeps the
    /// lease (and therefore the execution slot) allocated.
    pub cancel_confirmed: bool,
}

/// Backoff before the attempt after `attempt`.
///
/// `enqueue` caps `max_attempts` at four, so the first three delays are the only
/// reachable ones; the fallback keeps an inconsistent row from panicking.
fn retry_delay(attempt: i32) -> Duration {
    let index = (attempt.max(1) - 1) as usize;
    let seconds = RETRY_BACKOFF_SECONDS
        .get(index)
        .copied()
        .unwrap_or_else(|| RETRY_BACKOFF_SECONDS[RETRY_BACKOFF_SECONDS.len() - 1]);
    Duration::seconds(seconds)
}

/// Requests cancellation of a job.
///
/// - `queued` and `retry_wait`: the job has no lease, so it becomes `cancelled`
///   immediately.
/// - `running`: it becomes `cancel_requested` and keeps its lease. The worker
///   must stop the execution, wait for it to exit and then call
///   [`finish_execution`] with `cancel_confirmed = true`.
/// - already `cancel_requested` or terminal: returned unchanged, so a repeated
///   request is idempotent.
pub async fn request_cancel(ctx: &AppContext, job_id: Uuid) -> Result<Job, DomainJobError> {
    request_cancel_inner(ctx, job_id, false).await
}

/// Browser cancellation reports conflicts for jobs that were already terminal.
/// A repeated request remains valid while the worker is confirming exit.
pub async fn request_active_cancel(ctx: &AppContext, job_id: Uuid) -> Result<Job, DomainJobError> {
    request_cancel_inner(ctx, job_id, true).await
}

async fn request_cancel_inner(
    ctx: &AppContext,
    job_id: Uuid,
    require_active: bool,
) -> Result<Job, DomainJobError> {
    // The conditional update can miss when the row is already cancelling or
    // terminal, but it can also miss a `queued` job that a worker claimed a
    // moment earlier. In that case the read below sees `running` and the request
    // is applied again, so a cancellation is not silently lost in the race with
    // a claim.
    for _ in 0..2 {
        if let Some(job) = repository::request_cancel(ctx.pool(), job_id).await? {
            return Ok(job);
        }
        let job = repository::load(ctx.pool(), job_id)
            .await?
            .ok_or(DomainJobError::NotFound(job_id))?;
        match JobState::parse(&job.state) {
            Some(JobState::Queued | JobState::RetryWait | JobState::Running) => continue,
            _ => return check_cancel_state(job, require_active),
        }
    }
    let job = repository::load(ctx.pool(), job_id)
        .await?
        .ok_or(DomainJobError::NotFound(job_id))?;
    // The retries above are a race-tightening measure, not a proof: if the row
    // keeps changing under us, fail instead of pretending that a still running
    // (or still queued) job was cancelled.
    match JobState::parse(&job.state) {
        Some(JobState::Queued | JobState::RetryWait | JobState::Running) => {
            Err(DomainJobError::CancelContended {
                job_id,
                state: job.state,
            })
        }
        _ => check_cancel_state(job, require_active),
    }
}

fn check_cancel_state(job: Job, require_active: bool) -> Result<Job, DomainJobError> {
    if require_active && JobState::parse(&job.state).is_some_and(JobState::is_terminal) {
        return Err(DomainJobError::CancelTerminal {
            job_id: job.id,
            state: job.state,
        });
    }
    Ok(job)
}

/// Records the result of one attempt, applying the retry policy.
///
/// The token must still own the job, otherwise [`DomainJobError::LeaseLost`] is
/// returned and the row is left untouched: a worker that lost its lease must
/// discard its result instead of overwriting the new owner's work.
///
/// A `cancel_requested` job never becomes `succeeded`: without a confirmed child
/// exit it stays `cancel_requested` with its lease, and with confirmation it
/// becomes `cancelled`.
///
/// The expected state is part of every write instead of being trusted from the
/// read above it: a cancellation request that lands between the read and the
/// write makes the write affect zero rows, and the row is then returned as a
/// still-pending cancellation. That is what stops a stale success or retry from
/// overwriting the cancellation intent.
pub async fn finish_execution(
    ctx: &AppContext,
    input: FinishExecution,
) -> Result<Job, DomainJobError> {
    let job_id = input.job_id;
    let job = repository::load(ctx.pool(), job_id)
        .await?
        .ok_or(DomainJobError::NotFound(job_id))?;

    if job.lease_token != Some(input.lease_token) {
        return Err(DomainJobError::LeaseLost { job_id });
    }
    // The token matched, so the row is `running`/`cancel_requested` and the
    // other lease columns exist; `Job::lease` still returns an option because it
    // is also used on rows read outside a lease check.
    let lease = job.lease().ok_or(DomainJobError::LeaseLost { job_id })?;

    // A requested cancellation keeps the lease until the worker confirms exit.
    // Check the persisted state as part of that write.
    if input.cancel_confirmed {
        return confirm_cancel(ctx, &lease, input.lease_token).await;
    }

    // A handler can stop a stale input on its own. An explicit request arriving
    // before this write still wins and must go through exit confirmation.
    if input.outcome == ExecutionOutcome::Cancelled {
        return finish_terminal(ctx, &lease, JobOutcome::Cancelled, &[JobState::Running]).await;
    }

    // Normal outcomes only replace a `running` attempt. If a cancellation
    // request arrived after the read above, the write matches zero rows and the
    // cancellation intent wins.
    let from_running = [JobState::Running];
    match &input.outcome {
        ExecutionOutcome::Succeeded => {
            finish_terminal(ctx, &lease, JobOutcome::Succeeded, &from_running).await
        }
        ExecutionOutcome::PermanentFailure { message } => {
            finish_terminal(
                ctx,
                &lease,
                JobOutcome::Failed {
                    error: message.clone(),
                },
                &from_running,
            )
            .await
        }
        ExecutionOutcome::TemporaryFailure { message } => {
            if job.attempts < job.max_attempts {
                let available_at = ctx.now() + retry_delay(job.attempts);
                if !repository::schedule_retry(
                    ctx.pool(),
                    &lease,
                    message,
                    available_at,
                    &from_running,
                )
                .await?
                {
                    return reconcile_cancelled(ctx, &lease).await;
                }
                // Re-read the row so the caller sees `retry_wait` and the exact
                // `available_at` that was persisted.
                repository::load(ctx.pool(), job_id)
                    .await?
                    .ok_or(DomainJobError::NotFound(job_id))
            } else {
                // The attempt budget is spent. The last failure is the one that
                // explains the terminal state.
                finish_terminal(
                    ctx,
                    &lease,
                    JobOutcome::Failed {
                        error: message.clone(),
                    },
                    &from_running,
                )
                .await
            }
        }
        ExecutionOutcome::Cancelled => unreachable!("handled above"),
    }
}

/// Manually retries a terminal job.
///
/// Only `failed` and `cancelled` jobs may be retried. The original row is never
/// modified: a new job with the same kind and payload is created and linked back
/// through `retry_of`. Reusing the idempotency key would return the original
/// row instead of creating new work, so the retry is unkeyed.
///
/// Retrying a `running`, `queued`, `retry_wait` or `cancel_requested` job is
/// rejected: that work is still pending or in flight and duplicating it would
/// run the same input twice.
pub async fn retry_failed(ctx: &AppContext, job_id: Uuid) -> Result<Job, DomainJobError> {
    let original = repository::load(ctx.pool(), job_id)
        .await?
        .ok_or(DomainJobError::NotFound(job_id))?;

    let state = JobState::parse(&original.state)
        .ok_or_else(|| DbError::UnknownJobState(original.state.clone()))?;
    let retired: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM outbox WHERE topic='restore.retired_job' AND job_id=$1)",
    )
    .bind(job_id)
    .fetch_one(ctx.pool())
    .await
    .map_err(DbError::from)?;
    if retired || !matches!(state, JobState::Failed | JobState::Cancelled) {
        return Err(DomainJobError::NotRetryable {
            job_id,
            state: original.state,
        });
    }

    let mut tx = ctx.pool().begin().await.map_err(DbError::from)?;
    if original.kind == scanner::SCAN_JOB_KIND {
        scanner::lock_scan_enqueue(&mut tx)
            .await
            .map_err(DbError::from)?;
        if scanner::scan_pending(&mut tx)
            .await
            .map_err(DbError::from)?
        {
            return Err(DomainJobError::ScanPending);
        }
    }
    let created = enqueue(
        &mut tx,
        JobSpec {
            kind: original.kind.clone(),
            idempotency_key: None,
            payload: original.payload.clone(),
            priority: original.priority,
            max_attempts: original.max_attempts,
            available_at: None,
        },
    )
    .await?;
    let retried = repository::link_retry(&mut tx, created.job.id, original.id).await?;
    tx.commit().await.map_err(DbError::from)?;

    Ok(retried)
}

/// Confirms a requested cancellation and releases the lease.
///
/// Only `cancel_requested` is accepted: a confirmation without a request is a
/// caller error, not a way to cancel a running job that never asked to stop.
async fn confirm_cancel(
    ctx: &AppContext,
    lease: &Lease,
    lease_token: Uuid,
) -> Result<Job, DomainJobError> {
    let job_id = lease.job_id;
    let cancelled = repository::finish_with_token(
        ctx.pool(),
        lease,
        JobOutcome::Cancelled,
        &[JobState::CancelRequested],
    )
    .await?;
    if cancelled {
        return repository::load(ctx.pool(), job_id)
            .await?
            .ok_or(DomainJobError::NotFound(job_id));
    }

    // Zero rows: the token moved, the request was never made, or the job is
    // already terminal. Re-read to tell those apart instead of guessing.
    let current = repository::load(ctx.pool(), job_id)
        .await?
        .ok_or(DomainJobError::NotFound(job_id))?;
    if current.lease_token != Some(lease_token) {
        return Err(DomainJobError::LeaseLost { job_id });
    }
    match JobState::parse(&current.state) {
        // The request exists and the lease is still ours: keep the slot and let
        // a later attempt confirm the exit.
        Some(JobState::CancelRequested) => Ok(current),
        Some(JobState::Running) => Err(DomainJobError::CancelNotRequested {
            job_id,
            state: current.state,
        }),
        _ => Err(DomainJobError::LeaseLost { job_id }),
    }
}

/// Writes a terminal state if the token still owns a job in `from_states`,
/// then re-reads it.
async fn finish_terminal(
    ctx: &AppContext,
    lease: &Lease,
    outcome: JobOutcome,
    from_states: &[JobState],
) -> Result<Job, DomainJobError> {
    if !repository::finish_with_token(ctx.pool(), lease, outcome, from_states).await? {
        return reconcile_cancelled(ctx, lease).await;
    }
    repository::load(ctx.pool(), lease.job_id)
        .await?
        .ok_or(DomainJobError::NotFound(lease.job_id))
}

/// Turns a zero-row write into the right answer.
///
/// When the token still owns the job and it is `cancel_requested`, a concurrent
/// cancellation arrived first: return the row unchanged so the caller keeps the
/// lease and the slot until the exit is confirmed. Every other reason means the
/// lease is no longer usable.
async fn reconcile_cancelled(ctx: &AppContext, lease: &Lease) -> Result<Job, DomainJobError> {
    let job_id = lease.job_id;
    let Some(current) = repository::load(ctx.pool(), job_id).await? else {
        return Err(DomainJobError::NotFound(job_id));
    };
    if current.lease_token == Some(lease.token)
        && current.state == JobState::CancelRequested.as_str()
    {
        return Ok(current);
    }
    Err(DomainJobError::LeaseLost { job_id })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListJobs {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub state: Option<JobState>,
}

pub struct JobPage {
    pub items: Vec<Job>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ListJobsError {
    #[error("invalid job list parameters")]
    Invalid,
    #[error("jobs unavailable")]
    Unavailable,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ListCursor {
    version: u8,
    scope: String,
    state: Option<JobState>,
    created_at: DateTime<Utc>,
    id: Uuid,
}

/// Reads the dispatch kind without loading a job's private payload.
pub async fn kind(ctx: &AppContext, job_id: Uuid) -> Result<Option<String>, DbError> {
    Ok(sqlx::query_scalar("SELECT kind FROM jobs WHERE id=$1")
        .bind(job_id)
        .fetch_optional(ctx.pool())
        .await?)
}

/// Lists the newest executions of the selected kinds, including terminal attempts.
pub async fn list(
    ctx: &AppContext,
    input: ListJobs,
    kinds: &[&str],
) -> Result<JobPage, ListJobsError> {
    let limit = input.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ListJobsError::Invalid);
    }
    let cursor = input
        .cursor
        .map(|raw| {
            if raw.len() > 2048 {
                return Err(ListJobsError::Invalid);
            }
            let decoded = URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| ListJobsError::Invalid)?;
            let cursor: ListCursor =
                serde_json::from_slice(&decoded).map_err(|_| ListJobsError::Invalid)?;
            if cursor.version != 1 || cursor.scope != "jobs" || cursor.state != input.state {
                return Err(ListJobsError::Invalid);
            }
            Ok(cursor)
        })
        .transpose()?;
    let mut items: Vec<Job> = sqlx::query_as(
        "SELECT * FROM jobs WHERE ($1::text IS NULL OR state=$1) \
         AND kind=ANY($5::text[]) AND ($2::timestamptz IS NULL OR (created_at,id) < ($2,$3)) \
         ORDER BY created_at DESC,id DESC LIMIT $4",
    )
    .bind(input.state.map(JobState::as_str))
    .bind(cursor.as_ref().map(|value| value.created_at))
    .bind(cursor.as_ref().map(|value| value.id))
    .bind(i64::from(limit) + 1)
    .bind(kinds)
    .fetch_all(ctx.pool())
    .await
    .map_err(|_| ListJobsError::Unavailable)?;

    let has_next = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if has_next {
        let last = items.last().ok_or(ListJobsError::Unavailable)?;
        let bytes = serde_json::to_vec(&ListCursor {
            version: 1,
            scope: "jobs".into(),
            state: input.state,
            created_at: last.created_at,
            id: last.id,
        })
        .map_err(|_| ListJobsError::Unavailable)?;
        Some(URL_SAFE_NO_PAD.encode(bytes))
    } else {
        None
    };
    Ok(JobPage { items, next_cursor })
}
