//! Durable background jobs: the `jobs` table, its outbox and the lease
//! primitives workers use to claim and finish a job safely.
//!
//! This module owns the primitive layer only:
//!
//! - [`enqueue`] writes a job (idempotently) plus one outbox row in the caller's
//!   transaction;
//! - [`claim`] hands a due job to one worker with a lease token;
//! - [`heartbeat`] extends that lease while the work runs;
//! - [`finish_with_token`] commits an outcome only if the caller still holds the
//!   lease.
//!
//! Retry policy, backoff and the terminal-state rules live in [`service`].
//! Nothing here decides whether a failed attempt is retried, and nothing here
//! assumes that a job is processed once: a worker that lost its lease must
//! discard its result.

mod repository;
pub mod service;

pub use repository::{
    claim, enqueue, finish_with_token, heartbeat, heartbeat_state, schedule_retry,
};
pub use service::{DomainJobError, ExecutionOutcome, FinishExecution};

use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

/// How long one claim owns a job before another worker may take it over.
pub const LEASE_SECONDS: i64 = 90;

/// How often a worker must call [`heartbeat`] to keep its lease.
///
/// A third of [`LEASE_SECONDS`], so two heartbeats may be skipped or delayed
/// before another worker is allowed to take the job over.
pub const HEARTBEAT_SECONDS: i64 = 20;

/// Attempts a job gets, including the first one, when the caller does not say.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 4;

/// A job to enqueue.
///
/// [`JobSpec::default`] describes a single-attempt-anonymous job; callers
/// normally set at least `kind` and `payload`.
#[derive(Debug, Clone, PartialEq)]
pub struct JobSpec {
    /// Work type, for example `parse` or `embed`.
    pub kind: String,
    /// Deduplication key, scoped to `kind`. `None` means "always a new job".
    pub idempotency_key: Option<String>,
    /// Work input; the schema belongs to the job kind, not to this table.
    pub payload: Value,
    /// Higher values are claimed first.
    pub priority: i32,
    /// Attempts including the first one; `1..=DEFAULT_MAX_ATTEMPTS`.
    pub max_attempts: i32,
    /// Earliest claim time; `None` means "as soon as possible".
    pub available_at: Option<DateTime<Utc>>,
}

impl Default for JobSpec {
    fn default() -> Self {
        JobSpec {
            kind: String::new(),
            idempotency_key: None,
            payload: Value::Object(Default::default()),
            priority: 0,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            available_at: None,
        }
    }
}

impl JobSpec {
    /// A job of `kind` with the default attempt budget and no delay.
    pub fn new(kind: impl Into<String>) -> Self {
        JobSpec {
            kind: kind.into(),
            ..JobSpec::default()
        }
    }

    /// Outbox topic published when this job is enqueued.
    ///
    /// One topic per kind so consumers can subscribe to a work type without
    /// reading the payload of every event.
    pub fn enqueued_topic(&self) -> String {
        enqueued_topic(&self.kind)
    }
}

/// Outbox topic for a newly enqueued job of `kind`.
pub fn enqueued_topic(kind: &str) -> String {
    format!("job.enqueued.{kind}")
}

/// Result of [`enqueue`].
///
/// `created = false` means the `(kind, idempotency_key)` pair already existed
/// and `job` is the row that was already there. That row may already be in a
/// terminal state (`succeeded`, `failed`, `cancelled`), so a caller that treats
/// "enqueue returned" as "the work will run" is wrong: it must inspect
/// `job.state` (or `created`) and decide whether to wait, report the existing
/// result or create a new job with a new key. T08 builds the retry flow on this
/// distinction.
#[derive(Debug, Clone, PartialEq)]
pub struct EnqueueOutcome {
    /// The stored job: newly inserted when `created`, otherwise the existing one.
    pub job: Job,
    /// Whether this call inserted the job (and therefore its outbox event).
    pub created: bool,
}

/// The seven states a job can be in.
///
/// The database column stays textual (T07 exposes [`Job::state`] as a `String`),
/// but code that branches on the state machine uses this enum so a typo cannot
/// silently take the wrong branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Failed,
    CancelRequested,
    Cancelled,
}

impl JobState {
    /// Database value of this state.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::RetryWait => "retry_wait",
            JobState::Succeeded => "succeeded",
            JobState::Failed => "failed",
            JobState::CancelRequested => "cancel_requested",
            JobState::Cancelled => "cancelled",
        }
    }

    /// Parses a database value, or `None` when it is not one of the seven
    /// states the check constraint allows.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => JobState::Queued,
            "running" => JobState::Running,
            "retry_wait" => JobState::RetryWait,
            "succeeded" => JobState::Succeeded,
            "failed" => JobState::Failed,
            "cancel_requested" => JobState::CancelRequested,
            "cancelled" => JobState::Cancelled,
            _ => return None,
        })
    }

    /// Whether no further attempt will be made without a manual retry.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A persisted job row.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub idempotency_key: Option<String>,
    pub payload: Value,
    pub state: String,
    pub priority: i32,
    pub attempts: i32,
    pub max_attempts: i32,
    pub available_at: DateTime<Utc>,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub locked_by: Option<String>,
    pub last_error: Option<String>,
    /// Set when this job was created by a manual retry of another job.
    pub retry_of: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Job {
    /// The lease a worker holds on this job, if any.
    ///
    /// `None` for every state that must not carry one (`queued`, `retry_wait`
    /// and the terminal states), which the database also enforces.
    pub fn lease(&self) -> Option<Lease> {
        Some(Lease {
            job_id: self.id,
            token: self.lease_token?,
            expires_at: self.lease_expires_at?,
            attempt: self.attempts,
            worker: self.locked_by.clone()?,
            state: JobState::parse(&self.state)?,
        })
    }

    /// Outbox payload for this job.
    pub fn outbox_payload(&self) -> Value {
        json!({ "job_id": self.id, "kind": self.kind })
    }
}

/// One successful claim: the job, the token that proves ownership and the
/// attempt number this claim started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub job_id: Uuid,
    pub token: Uuid,
    pub expires_at: DateTime<Utc>,
    /// 1 for the first claim, 2 after one takeover, and so on.
    pub attempt: i32,
    pub worker: String,
    /// State the job was in when it was claimed.
    ///
    /// A takeover of a `cancel_requested` job repeats the attempt but must not
    /// execute it: the new holder has to confirm the cancellation instead.
    pub state: JobState,
}

/// Result a worker reports through [`finish_with_token`].
///
/// `retry_wait` is not represented here: scheduling a retry is a state-machine
/// decision (T08) that must also compute the backoff delay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    Succeeded,
    Failed { error: String },
    Cancelled,
}

impl JobOutcome {
    /// Database value written to `jobs.state`.
    pub fn state(&self) -> &'static str {
        match self {
            JobOutcome::Succeeded => "succeeded",
            JobOutcome::Failed { .. } => "failed",
            JobOutcome::Cancelled => "cancelled",
        }
    }

    /// Value written to `jobs.last_error`.
    pub fn last_error(&self) -> Option<&str> {
        match self {
            JobOutcome::Succeeded | JobOutcome::Cancelled => None,
            JobOutcome::Failed { error } => Some(error),
        }
    }
}

/// Read the dispatch envelope only while this attempt owns a live lease.
/// Publication must still recheck inside its own domain transaction.
pub async fn load_claimed(
    pool: &sqlx::PgPool,
    lease: &Lease,
) -> Result<Option<Job>, crate::db::DbError> {
    Ok(sqlx::query_as("SELECT * FROM jobs WHERE id=$1 AND lease_token=$2 AND lease_expires_at>clock_timestamp() AND state='running'")
        .bind(lease.job_id).bind(lease.token).fetch_optional(pool).await?)
}
