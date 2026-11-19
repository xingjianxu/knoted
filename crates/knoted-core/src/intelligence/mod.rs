//! The internal execution protocol between the Rust worker and the Python
//! intelligence service.
//!
//! The authoritative contract is `api/intelligence.openapi.yaml`; the types here
//! mirror it field for field (`snake_case`, `additionalProperties: false`, so
//! unknown fields are rejected). The contract has no separate "execution token"
//! field, and none is invented: the equivalent guarantee is the triple the
//! record must echo — `input_sha256`, `source_version` and `options_revision` —
//! plus the digest and size of the staged artifact, all verified before a
//! result is accepted.
//!
//! Two rules make the client safe to trust:
//!
//! - :meth:`ExecutionRecord::verify_fresh` rejects a computation that finished
//!   after its deadline (or is still running past it), so a stale result can
//!   never be published as current;
//! - :meth:`ExecutionResult::verify_against` recomputes the artifact's SHA-256
//!   and size from staging instead of believing the service's numbers.

pub mod client;

pub use client::{CANCEL_TIMEOUT, CreateOutcome, DEFAULT_TIMEOUT, IntelligenceClient};

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::files::staging::{ObjectId, Sha256Hex, StagedObject, StagingError, StagingStore};

/// Work the Python service can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    /// Extract structured text from a document.
    Parse,
    /// Produce embeddings for already parsed text.
    Embed,
}

impl fmt::Display for ExecutionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExecutionKind::Parse => "parse",
            ExecutionKind::Embed => "embed",
        })
    }
}

/// Durable execution state.
///
/// `interrupted` is a *retryable terminal* state: the service restarted while
/// the execution was in flight, so the worker must create a new
/// `execution_id` instead of treating the old one as still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// Accepted, waiting for a slot.
    Queued,
    /// A child process is computing.
    Running,
    /// Finished with a publishable result.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Cancellation requested; the child has not confirmed its exit yet.
    CancelRequested,
    /// Cancelled, with the child exit confirmed.
    Cancelled,
    /// The service restarted while the execution was in flight.
    Interrupted,
}

impl ExecutionState {
    /// Whether the execution can no longer change.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ExecutionState::Succeeded
                | ExecutionState::Failed
                | ExecutionState::Cancelled
                | ExecutionState::Interrupted
        )
    }

    /// Whether the execution produced a result.
    pub fn is_success(self) -> bool {
        matches!(self, ExecutionState::Succeeded)
    }

    /// Whether the child is confirmed gone.
    ///
    /// Only `cancelled` is a confirmation. `cancel_requested` means the group
    /// was signalled but has not exited, and `interrupted` means the service
    /// died; neither may release a slot or a budget.
    pub fn cancellation_confirmed(self) -> bool {
        matches!(self, ExecutionState::Cancelled)
    }
}

impl fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExecutionState::Queued => "queued",
            ExecutionState::Running => "running",
            ExecutionState::Succeeded => "succeeded",
            ExecutionState::Failed => "failed",
            ExecutionState::CancelRequested => "cancel_requested",
            ExecutionState::Cancelled => "cancelled",
            ExecutionState::Interrupted => "interrupted",
        })
    }
}

/// One invalid field inside an [`ApiError`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldError {
    /// Request field that failed validation.
    pub field: String,
    /// Human-readable explanation; branch on the code, not on this text.
    pub message: String,
}

/// The error envelope every internal endpoint returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Id of the failing request, as returned by the service.
    pub request_id: Uuid,
    /// Field-level failures, when the code is `validation_failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_errors: Option<Vec<FieldError>>,
    /// Current version, when a `409` is a version conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_version: Option<i64>,
}

/// Request body of `POST /internal/v1/executions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    /// Worker-chosen execution id; a retry uses a new one.
    pub execution_id: Uuid,
    /// Kind of work.
    pub kind: ExecutionKind,
    /// Controlled staging key of the input.
    pub input_object_id: ObjectId,
    /// Digest the service must confirm before computing.
    pub input_sha256: Sha256Hex,
    /// Version of the source content the input was read from.
    pub source_version: i64,
    /// Instant after which the result is no longer publishable.
    pub deadline: DateTime<Utc>,
    /// Revision of the parse/embed options.
    pub options_revision: String,
}

/// Durable record of one execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "UncheckedExecutionRecord")]
pub struct ExecutionRecord {
    /// Execution id the worker chose.
    pub execution_id: Uuid,
    /// Kind of work.
    pub kind: ExecutionKind,
    /// Current state.
    pub state: ExecutionState,
    /// Whole supervised compute tree is durably confirmed gone. Missing legacy proof is false.
    #[serde(default)]
    pub compute_exit_confirmed: bool,
    /// Digest echoed from the request.
    pub input_sha256: Sha256Hex,
    /// Source version echoed from the request.
    pub source_version: i64,
    /// Options revision echoed from the request.
    pub options_revision: String,
    /// When the execution was accepted.
    pub created_at: DateTime<Utc>,
    /// When the record last changed.
    pub updated_at: DateTime<Utc>,
    /// When a slot was taken and the child started.
    pub started_at: Option<DateTime<Utc>>,
    /// When the execution reached a terminal state.
    pub finished_at: Option<DateTime<Utc>>,
    /// Diagnostic for a non-success terminal state.
    pub error: Option<ApiError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedExecutionRecord {
    /// Execution id the worker chose.
    execution_id: Uuid,
    /// Kind of work.
    kind: ExecutionKind,
    /// Current state.
    state: ExecutionState,
    /// Whole supervised compute tree is durably confirmed gone. Missing legacy proof is false.
    #[serde(default)]
    compute_exit_confirmed: bool,
    /// Digest echoed from the request.
    input_sha256: Sha256Hex,
    /// Source version echoed from the request.
    source_version: i64,
    /// Options revision echoed from the request.
    options_revision: String,
    /// When the execution was accepted.
    created_at: DateTime<Utc>,
    /// When the record last changed.
    updated_at: DateTime<Utc>,
    /// When a slot was taken and the child started.
    started_at: Option<DateTime<Utc>>,
    /// When the execution reached a terminal state.
    finished_at: Option<DateTime<Utc>>,
    /// Diagnostic for a non-success terminal state.
    error: Option<ApiError>,
}
impl TryFrom<UncheckedExecutionRecord> for ExecutionRecord {
    type Error = String;
    fn try_from(raw: UncheckedExecutionRecord) -> Result<Self, Self::Error> {
        if raw.compute_exit_confirmed && !raw.state.is_terminal() {
            return Err("compute_exit_confirmed requires a terminal state".to_owned());
        }
        Ok(Self {
            execution_id: raw.execution_id,
            kind: raw.kind,
            state: raw.state,
            compute_exit_confirmed: raw.compute_exit_confirmed,
            input_sha256: raw.input_sha256,
            source_version: raw.source_version,
            options_revision: raw.options_revision,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            started_at: raw.started_at,
            finished_at: raw.finished_at,
            error: raw.error,
        })
    }
}

impl ExecutionRecord {
    /// Resource proof is independent of whether a cancellation command won.
    pub fn verify_resource_exit(
        &self,
        request: &ExecutionRequest,
    ) -> Result<(), IntelligenceError> {
        self.verify_echo(request)?;
        if !self.state.is_terminal() || !self.compute_exit_confirmed {
            return Err(IntelligenceError::CancelUnconfirmed {
                execution_id: self.execution_id,
            });
        }
        Ok(())
    }

    /// Checks that the record describes the request the caller sent.
    ///
    /// The contract has no separate execution token, so this echo — together
    /// with the artifact digest verified in
    /// [`ExecutionResult::verify_against`] — is what ties a result to the input
    /// version it was computed from. A mismatch means the service answered about
    /// a different execution, and the result must be discarded.
    pub fn verify_echo(&self, request: &ExecutionRequest) -> Result<(), IntelligenceError> {
        if self.execution_id != request.execution_id {
            return Err(IntelligenceError::EchoMismatch {
                field: "execution_id",
                expected: request.execution_id.to_string(),
                actual: self.execution_id.to_string(),
            });
        }
        if self.kind != request.kind {
            return Err(IntelligenceError::EchoMismatch {
                field: "kind",
                expected: request.kind.to_string(),
                actual: self.kind.to_string(),
            });
        }
        if self.input_sha256 != request.input_sha256 {
            return Err(IntelligenceError::EchoMismatch {
                field: "input_sha256",
                expected: request.input_sha256.as_str().to_owned(),
                actual: self.input_sha256.as_str().to_owned(),
            });
        }
        if self.source_version != request.source_version {
            return Err(IntelligenceError::EchoMismatch {
                field: "source_version",
                expected: request.source_version.to_string(),
                actual: self.source_version.to_string(),
            });
        }
        if self.options_revision != request.options_revision {
            return Err(IntelligenceError::EchoMismatch {
                field: "options_revision",
                expected: request.options_revision.clone(),
                actual: self.options_revision.clone(),
            });
        }
        Ok(())
    }

    /// Rejects a result that the deadline says is no longer current.
    ///
    /// A finished execution past its deadline is expired, and an execution that
    /// is still unfinished past its deadline can no longer produce a publishable
    /// result. `now` is passed in so the caller uses its injected clock.
    pub fn verify_fresh(
        &self,
        request: &ExecutionRequest,
        now: DateTime<Utc>,
    ) -> Result<(), IntelligenceError> {
        let deadline = request.deadline;
        match self.finished_at {
            Some(finished_at) if finished_at > deadline => Err(IntelligenceError::Expired {
                deadline,
                finished_at: Some(finished_at),
                now,
            }),
            None if now > deadline => Err(IntelligenceError::Expired {
                deadline,
                finished_at: None,
                now,
            }),
            _ => Ok(()),
        }
    }
}

/// Artifact manifest returned by `GET /internal/v1/executions/{id}/result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    /// Controlled staging key of the artifact.
    pub object_id: ObjectId,
    /// Digest the service registered.
    pub sha256: Sha256Hex,
    /// Size the service registered.
    pub size: u64,
    /// Media type of the artifact.
    pub media_type: String,
    /// Descriptive metadata; every value is a string on the wire.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl ExecutionResult {
    /// Recomputes the artifact from staging and compares it with the manifest.
    ///
    /// The service's numbers are never trusted directly: an artifact that was
    /// rewritten, truncated or replaced after the service registered it fails
    /// here with [`StagingError::DigestMismatch`] or
    /// [`StagingError::SizeMismatch`].
    pub fn verify_against(&self, store: &StagingStore) -> Result<StagedObject, IntelligenceError> {
        if self.media_type.trim().is_empty() {
            return Err(IntelligenceError::MalformedResponse {
                reason: "result media_type is empty".to_owned(),
            });
        }
        let expected = StagedObject {
            object_id: self.object_id.clone(),
            sha256: self.sha256.clone(),
            size: self.size,
        };
        Ok(store.verify(&expected)?)
    }
}

/// Health of the internal service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthStatus {
    /// Overall status.
    pub status: HealthState,
    /// Executions currently in flight.
    pub active_executions: u64,
}

/// Overall service status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// Everything is fine.
    Ok,
    /// A dependency is degraded (for example an unconfirmed cancellation).
    Degraded,
}

/// Everything the client can refuse.
#[derive(Debug, thiserror::Error)]
pub enum IntelligenceError {
    #[error("execution submission ledger unavailable")]
    SubmissionLedger,
    /// The request never produced a response.
    #[error("intelligence service transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// The service answered with an error envelope.
    #[error("intelligence service rejected the request: {message}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// Stable error code, when the body carried one.
        code: Option<String>,
        /// Human-readable message.
        message: String,
    },
    /// The service answered with a status this operation does not accept.
    #[error("intelligence service returned {status} for {operation}")]
    UnexpectedStatus {
        /// Operation name, for logs.
        operation: &'static str,
        /// HTTP status code.
        status: u16,
    },
    /// A cancellation request timed out, so the exit is **not** confirmed.
    ///
    /// The HTTP answer was lost; the computation may still be running. The
    /// caller must poll `GET /executions/{id}` (see
    /// [`ExecutionState::cancellation_confirmed`]) before releasing anything.
    #[error(
        "cancellation of execution {execution_id} was not confirmed before the request \
         timed out; poll the record instead"
    )]
    CancelUnconfirmed {
        /// Execution whose cancellation is uncertain.
        execution_id: Uuid,
    },
    /// The response body does not match the contract.
    #[error("intelligence service response is not the contract: {reason}")]
    MalformedResponse {
        /// What went wrong.
        reason: String,
    },
    /// A staging artifact failed verification.
    #[error("staged artifact failed verification: {0}")]
    Staging(#[from] StagingError),
    /// The record does not describe the request.
    #[error("execution {field} does not match the request: expected {expected}, got {actual}")]
    EchoMismatch {
        /// Field that differs.
        field: &'static str,
        /// Value the request carried.
        expected: String,
        /// Value the service returned.
        actual: String,
    },
    /// The deadline passed before the result could be published.
    #[error(
        "execution expired: deadline {deadline}, finished_at {finished_at:?}, observed at {now}"
    )]
    Expired {
        /// Deadline carried by the request.
        deadline: DateTime<Utc>,
        /// When the execution finished, if it did.
        finished_at: Option<DateTime<Utc>>,
        /// Instant the check used.
        now: DateTime<Utc>,
    },
    /// The execution is in a state the caller cannot proceed from.
    #[error("execution is {state}, expected {expected}")]
    UnexpectedState {
        /// State the service reported.
        state: ExecutionState,
        /// State the caller required.
        expected: &'static str,
    },
    /// The base URL cannot address an HTTP service.
    #[error("invalid intelligence base url {url:?}: {reason}")]
    InvalidBaseUrl {
        /// The rejected URL.
        url: String,
        /// Why it was rejected.
        reason: String,
    },
}
