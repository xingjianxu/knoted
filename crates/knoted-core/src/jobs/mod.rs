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
//! Retry policy, backoff and the terminal-state rules live in T08. Nothing here
//! decides whether a failed attempt is retried, and nothing here assumes that a
//! job is processed once: a worker that lost its lease must discard its result.

mod repository;

pub use repository::{claim, enqueue, finish_with_token, heartbeat};

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
    /// Attempts including the first one; never below 1.
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Job {
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
