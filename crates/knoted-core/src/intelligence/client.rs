//! HTTP client for the internal execution protocol.
//!
//! Only loopback traffic is expected (`http://127.0.0.1:15603` by default), but
//! the client still builds with rustls so no OpenSSL is linked into the worker.
//!
//! The client is deliberately literal: `cancel_execution` returns whatever the
//! service recorded, including `cancel_requested`, and only
//! [`ExecutionState::cancellation_confirmed`] says a child is really gone. An
//! HTTP timeout is a lost answer, never a cancelled computation, which is why a
//! timed-out cancel is reported as [`IntelligenceError::CancelUnconfirmed`]
//! instead of a transport error: the caller must poll the record.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::{
    ApiError, ExecutionRecord, ExecutionRequest, ExecutionResult, HealthStatus, IntelligenceError,
};
use crate::files::staging::StagingStore;

/// Per-request timeout.
///
/// Short on purpose: every endpoint is a bookkeeping call, and a parse that
/// takes minutes is observed by polling, not by one long request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for a cancel request.
///
/// Longer than [`DEFAULT_TIMEOUT`] on purpose: the service may spend the whole
/// SIGTERM grace period plus the SIGKILL budget before it answers. Even so, a
/// timeout only means "not confirmed" — never "cancelled".
pub const CANCEL_TIMEOUT: Duration = Duration::from_secs(30);

const EXECUTIONS_PATH: &str = "/internal/v1/executions";
const HEALTH_PATH: &str = "/internal/v1/health";
const USER_AGENT: &str = concat!("knoted-worker/", env!("CARGO_PKG_VERSION"));

/// Result of a create call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOutcome {
    /// The stored record.
    pub record: ExecutionRecord,
    /// `true` for `201 Created`, `false` when an identical request was replayed
    /// (`200`) and the original computation was left alone.
    pub created: bool,
}

/// Typed client for the Python service.
#[derive(Debug, Clone)]
pub struct IntelligenceClient {
    submissions: Option<sqlx::PgPool>,
    base_url: String,
    http: reqwest::Client,
    cancel_timeout: Duration,
}

impl IntelligenceClient {
    /// Builds a client with the default timeout and rustls TLS.
    pub fn new(base_url: impl Into<String>) -> Result<IntelligenceClient, IntelligenceError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()?;
        IntelligenceClient::with_client(base_url, http)
    }

    /// Application clients register every outbound create, including idempotent
    /// resends, before dispatch. Standalone protocol probes have no database.
    pub fn from_context(ctx: &crate::context::AppContext) -> Result<Self, IntelligenceError> {
        let mut client = Self::new(&ctx.intelligence().base_url)?;
        client.submissions = Some(ctx.pool().clone());
        Ok(client)
    }

    /// Builds a client around an existing `reqwest::Client`.
    pub fn with_client(
        base_url: impl Into<String>,
        http: reqwest::Client,
    ) -> Result<IntelligenceClient, IntelligenceError> {
        let base_url = base_url.into();
        let trimmed = base_url.trim_end_matches('/');
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return Err(IntelligenceError::InvalidBaseUrl {
                url: base_url,
                reason: "the base url must start with http:// or https://".to_owned(),
            });
        }
        if trimmed.is_empty() {
            return Err(IntelligenceError::InvalidBaseUrl {
                url: base_url,
                reason: "the base url must not be empty".to_owned(),
            });
        }
        Ok(IntelligenceClient {
            submissions: None,
            base_url: trimmed.to_owned(),
            http,
            cancel_timeout: CANCEL_TIMEOUT,
        })
    }

    /// Overrides how long a cancel request waits for the service to answer.
    pub fn with_cancel_timeout(mut self, cancel_timeout: Duration) -> IntelligenceClient {
        self.cancel_timeout = cancel_timeout;
        self
    }

    /// The base url, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Submits an execution.
    ///
    /// `created == false` means the service replayed an identical request and
    /// did **not** start new work, so the caller must not wait for a second
    /// computation. The returned record is checked against the request before
    /// it is handed back.
    pub async fn create_execution(
        &self,
        request: &ExecutionRequest,
    ) -> Result<CreateOutcome, IntelligenceError> {
        let submission = Uuid::new_v4();
        if let Some(pool) = &self.submissions {
            let mut tx = pool
                .begin()
                .await
                .map_err(|_| IntelligenceError::SubmissionLedger)?;
            sqlx::query("INSERT INTO maintenance_external_targets VALUES ('python',$1,'') ON CONFLICT DO NOTHING")
                .bind(&self.base_url).execute(&mut *tx).await.map_err(|_| IntelligenceError::SubmissionLedger)?;
            sqlx::query("INSERT INTO maintenance_external_submissions (id,service,execution_request) VALUES ($1,'python',$2)")
                .bind(submission).bind(sqlx::types::Json(request)).execute(&mut *tx).await
                .map_err(|_| IntelligenceError::SubmissionLedger)?;
            tx.commit()
                .await
                .map_err(|_| IntelligenceError::SubmissionLedger)?;
        }
        let response = self
            .http
            .post(format!("{}{EXECUTIONS_PATH}", self.base_url))
            .json(request)
            .send()
            .await?;
        let created = match response.status() {
            StatusCode::CREATED => true,
            StatusCode::OK => false,
            _ => {
                let error = self.rejection("create_execution", response).await;
                let rejected = matches!(&error, IntelligenceError::Status {status, code:Some(code), ..}
                    if matches!((*status,code.as_str()), (409,"execution_conflict") | (422,"validation_failed") | (404,"not_found") | (405,"method_not_allowed")));
                if let Some(pool) = &self.submissions
                    && rejected
                {
                    sqlx::query("DELETE FROM maintenance_external_submissions WHERE id=$1")
                        .bind(submission)
                        .execute(pool)
                        .await
                        .map_err(|_| IntelligenceError::SubmissionLedger)?;
                }
                return Err(error);
            }
        };
        let record: ExecutionRecord = decode(response, "create_execution").await?;
        record.verify_echo(request)?;
        self.acknowledge_registration(request).await?;
        Ok(CreateOutcome { record, created })
    }

    async fn acknowledge_registration(
        &self,
        request: &ExecutionRequest,
    ) -> Result<(), IntelligenceError> {
        if let Some(pool) = &self.submissions {
            // Python durably registers the id before returning a matching echo.
            // A delayed duplicate of this exact request cannot start a second job.
            sqlx::query("DELETE FROM maintenance_external_submissions WHERE service='python' AND execution_request=$1")
                .bind(sqlx::types::Json(request)).execute(pool).await.map_err(|_| IntelligenceError::SubmissionLedger)?;
        }
        Ok(())
    }

    /// Reads the current record.
    pub async fn get_execution(
        &self,
        execution_id: Uuid,
    ) -> Result<ExecutionRecord, IntelligenceError> {
        let response = self
            .http
            .get(self.execution_url(execution_id))
            .send()
            .await?;
        self.expect_ok("get_execution", response).await
    }

    /// Requests cancellation and returns the record **as the service stored it**.
    ///
    /// A `cancel_requested` answer means the group was signalled; it is not an
    /// exit. Poll [`ExecutionState::cancellation_confirmed`] before releasing a
    /// slot or a concurrency budget.
    pub async fn cancel_execution(
        &self,
        execution_id: Uuid,
    ) -> Result<ExecutionRecord, IntelligenceError> {
        let response = self
            .http
            .post(format!("{}/cancel", self.execution_url(execution_id)))
            .timeout(self.cancel_timeout)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    // A lost answer is not a confirmation: the child may still be
                    // alive, so the caller has to observe the record itself.
                    IntelligenceError::CancelUnconfirmed { execution_id }
                } else {
                    IntelligenceError::Transport(error)
                }
            })?;
        self.expect_ok("cancel_execution", response).await
    }

    /// Reads the artifact manifest.
    ///
    /// The manifest is only meaningful once the record is `succeeded`; a
    /// `409`/`422` becomes [`IntelligenceError::Status`] with the service code.
    pub async fn get_result(
        &self,
        execution_id: Uuid,
    ) -> Result<ExecutionResult, IntelligenceError> {
        let response = self
            .http
            .get(format!("{}/result", self.execution_url(execution_id)))
            .send()
            .await?;
        self.expect_ok("get_result", response).await
    }

    /// Reads the service health.
    pub async fn health(&self) -> Result<HealthStatus, IntelligenceError> {
        let response = self
            .http
            .get(format!("{}{HEALTH_PATH}", self.base_url))
            .send()
            .await?;
        self.expect_ok("health", response).await
    }

    /// Reads a result and returns it only when everything checks out.
    ///
    /// The order matters: the record must echo the request, must not be
    /// expired, must be `succeeded`, and the artifact must hash to the digest
    /// and size the service registered. Any failure is an error, never a
    /// best-effort result.
    pub async fn fetch_verified_result(
        &self,
        request: &ExecutionRequest,
        store: &StagingStore,
        now: DateTime<Utc>,
    ) -> Result<ExecutionResult, IntelligenceError> {
        let record = self.get_execution(request.execution_id).await?;
        record.verify_echo(request)?;
        record.verify_fresh(request, now)?;
        if !record.state.is_success() {
            return Err(IntelligenceError::UnexpectedState {
                state: record.state,
                expected: "succeeded",
            });
        }
        record.verify_resource_exit(request)?;
        if record.finished_at.is_none() {
            return Err(IntelligenceError::MalformedResponse {
                reason: format!(
                    "execution {} is succeeded but has no finished_at",
                    record.execution_id
                ),
            });
        }
        let result = self.get_result(request.execution_id).await?;
        result.verify_against(store)?;
        Ok(result)
    }

    /// Fence a possibly late POST using its original persisted request, then
    /// cancel/observe. Only a matching terminal exit receipt returns success.
    pub async fn settle_execution(
        &self,
        request: &ExecutionRequest,
    ) -> Result<(), IntelligenceError> {
        let record = match self.get_execution(request.execution_id).await {
            Ok(record) => record,
            Err(IntelligenceError::Status { status: 404, .. }) => {
                self.create_execution(request).await?.record
            }
            Err(error) => return Err(error),
        };
        record.verify_echo(request)?;
        self.acknowledge_registration(request).await?;
        if record.verify_resource_exit(request).is_ok() {
            return Ok(());
        }
        match self.cancel_execution(request.execution_id).await {
            Ok(record) => {
                record.verify_echo(request)?;
                if record.verify_resource_exit(request).is_ok() {
                    return Ok(());
                }
            }
            Err(
                IntelligenceError::Status { status: 409, .. }
                | IntelligenceError::CancelUnconfirmed { .. },
            ) => {}
            Err(error) => return Err(error),
        }
        let deadline = tokio::time::Instant::now() + CANCEL_TIMEOUT;
        loop {
            let record = self.get_execution(request.execution_id).await?;
            record.verify_echo(request)?;
            if record.verify_resource_exit(request).is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(IntelligenceError::CancelUnconfirmed {
                    execution_id: request.execution_id,
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn execution_url(&self, execution_id: Uuid) -> String {
        format!("{}{EXECUTIONS_PATH}/{execution_id}", self.base_url)
    }

    async fn expect_ok<T>(
        &self,
        operation: &'static str,
        response: reqwest::Response,
    ) -> Result<T, IntelligenceError>
    where
        T: DeserializeOwned,
    {
        if response.status() != StatusCode::OK {
            return Err(self.rejection(operation, response).await);
        }
        decode(response, operation).await
    }

    /// Turns a non-success response into the most specific error available.
    async fn rejection(
        &self,
        operation: &'static str,
        response: reqwest::Response,
    ) -> IntelligenceError {
        let status = response.status();
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return IntelligenceError::Transport(error),
        };
        match serde_json::from_slice::<ApiError>(&bytes) {
            Ok(body) => IntelligenceError::Status {
                status: status.as_u16(),
                code: Some(body.code),
                message: format!("{operation}: {}", body.message),
            },
            Err(_) => IntelligenceError::Status {
                status: status.as_u16(),
                code: None,
                message: format!(
                    "{operation}: {}",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
                ),
            },
        }
    }
}

async fn decode<T>(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<T, IntelligenceError>
where
    T: DeserializeOwned,
{
    let bytes = response.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|error| IntelligenceError::MalformedResponse {
        reason: format!("{operation}: {error}"),
    })
}
