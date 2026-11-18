//! Browser job actions and a DTO that excludes payloads, leases and internal errors.
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    files::scanner::{self, ManualScan, ManualScanError},
    jobs::{
        DomainJobError, Job, JobState,
        service::{self, ListJobs, ListJobsError},
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::ApiErrorBody,
    middleware::{
        RequestId,
        auth::{CurrentSession, require_csrf, require_session},
    },
};

// The index panel can only manage ingestion/retrieval work. Other consumers,
// including chat generation, own their lifecycle and resources.
const INDEX_KINDS: &[&str] = &[
    scanner::SCAN_JOB_KIND,
    knoted_core::files::capture::CAPTURE_JOB_KIND,
    knoted_core::files::extraction::EXTRACT_JOB_KIND,
    knoted_core::search::meili::PUBLISH_JOB_KIND,
    knoted_core::search::meili::CLEANUP_JOB_KIND,
    "rag_chunks.build",
    "rag_chunks.publish",
    "rag_vectors.embed",
];

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/jobs", get(list))
        .route("/jobs/scan", post(scan))
        .route("/jobs/{job_id}/retry", post(retry))
        .route("/jobs/{job_id}/cancel", post(cancel))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}

#[derive(Serialize)]
struct JobDto {
    id: Uuid,
    kind: String,
    state: String,
    attempts: i32,
    max_attempts: i32,
    error: Option<&'static str>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl From<Job> for JobDto {
    fn from(job: Job) -> Self {
        let state = JobState::parse(&job.state);
        let error = match state {
            Some(JobState::Failed | JobState::RetryWait) => {
                Some(public_error(job.last_error.as_deref()))
            }
            _ => None,
        };
        Self {
            id: job.id,
            kind: job.kind,
            state: job.state,
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: state
                .filter(|state| state.is_terminal())
                .map(|_| job.updated_at),
        }
    }
}

impl JobDto {
    /// The scan POST returns the original submission response on replay. The
    /// list endpoint exposes the current state independently.
    fn scan_submission(job: Job) -> Self {
        Self {
            id: job.id,
            kind: job.kind,
            state: JobState::Queued.as_str().into(),
            attempts: 0,
            max_attempts: job.max_attempts,
            error: None,
            created_at: job.created_at,
            updated_at: job.created_at,
            started_at: None,
            finished_at: None,
        }
    }
}

/// Exact mappings only. The database can contain messages from old workers or
/// dependencies that we must not reflect to the browser.
fn public_error(error: Option<&str>) -> &'static str {
    match error {
        Some("unsupported_format") => "unsupported_format",
        Some("invalid or unsupported text encoding") => "invalid_text_encoding",
        Some("input_too_large" | "input exceeds 100 MiB") => "input_too_large",
        Some("image_too_large") => "image_too_large",
        Some("too_many_pages") => "too_many_pages",
        Some("office_budget_exceeded" | "artifact_too_large" | "output_too_large") => {
            "output_too_large"
        }
        Some("deadline_exceeded" | "text extraction exceeded 900 seconds") => "deadline_exceeded",
        Some("corrupt_input" | "invalid_input" | "empty_input") => "invalid_input",
        Some("file scanning disabled") => "file_scanning_disabled",
        Some(
            "file scan incomplete; sources retained" | "file scan unavailable; sources retained",
        ) => "scan_unavailable",
        Some("input_digest_mismatch" | "capture changed or unavailable") => "source_changed",
        Some("models_missing" | "artifacts_unset" | "snapshot_unset") => "model_unavailable",
        _ => "job_failed",
    }
}

#[derive(Serialize)]
struct JobPageDto {
    items: Vec<JobDto>,
    next_cursor: Option<String>,
}

async fn list(
    State(ctx): State<AppContext>,
    request: RequestId,
    query: Result<Query<ListJobs>, QueryRejection>,
) -> Response {
    let Ok(Query(input)) = query else {
        return bad(request);
    };
    match service::list(&ctx, input, INDEX_KINDS).await {
        Ok(page) => Json(JobPageDto {
            items: page.items.into_iter().map(JobDto::from).collect(),
            next_cursor: page.next_cursor,
        })
        .into_response(),
        Err(ListJobsError::Invalid) => bad(request),
        Err(ListJobsError::Unavailable) => unavailable(request),
    }
}

async fn require_index_job(ctx: &AppContext, job_id: Uuid) -> Result<(), DomainJobError> {
    let kind = service::kind(ctx, job_id).await?;
    if kind
        .as_deref()
        .is_some_and(|kind| INDEX_KINDS.contains(&kind))
    {
        Ok(())
    } else {
        Err(DomainJobError::NotFound(job_id))
    }
}

async fn retry(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(job_id)) = path else {
        return bad(request);
    };
    if let Err(error) = require_index_job(&ctx, job_id).await {
        return job_error(request, error);
    }
    match service::retry_failed(&ctx, job_id).await {
        Ok(job) => (StatusCode::CREATED, Json(JobDto::from(job))).into_response(),
        Err(error) => job_error(request, error),
    }
}

async fn cancel(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(job_id)) = path else {
        return bad(request);
    };
    if let Err(error) = require_index_job(&ctx, job_id).await {
        return job_error(request, error);
    }
    match service::request_active_cancel(&ctx, job_id).await {
        Ok(job) => Json(JobDto::from(job)).into_response(),
        Err(error) => job_error(request, error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanBody {
    #[serde(default)]
    force: bool,
}

async fn scan(
    State(ctx): State<AppContext>,
    Extension(session): Extension<CurrentSession>,
    request: RequestId,
    headers: HeaderMap,
    body: Result<Json<ScanBody>, JsonRejection>,
) -> Response {
    let mut keys = headers.get_all("idempotency-key").iter();
    let key = match (keys.next().and_then(|key| key.to_str().ok()), keys.next()) {
        (Some(key), None) if !key.trim().is_empty() && key.len() <= 255 => key.to_owned(),
        _ => return bad(request),
    };
    let Json(body) = match body {
        Ok(value) => value,
        Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return reply(
                request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "scan options exceed the body limit",
            );
        }
        Err(_) => return bad(request),
    };
    match scanner::request_scan(
        &ctx,
        ManualScan {
            owner_id: session.session.user_id,
            idempotency_key: key,
            force: body.force,
        },
    )
    .await
    {
        Ok(job) => (StatusCode::CREATED, Json(JobDto::scan_submission(job))).into_response(),
        Err(ManualScanError::Pending) => scan_pending(request),
        Err(ManualScanError::KeyConflict) => reply(
            request,
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "scan key already used with other options",
        ),
        Err(ManualScanError::Disabled) => reply(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "file_scanning_disabled",
            "file scanning is disabled",
        ),
        Err(ManualScanError::Unavailable) => unavailable(request),
    }
}

fn job_error(request: RequestId, error: DomainJobError) -> Response {
    match error {
        DomainJobError::NotFound(_) => {
            reply(request, StatusCode::NOT_FOUND, "not_found", "job not found")
        }
        DomainJobError::ScanPending => scan_pending(request),
        DomainJobError::Database(_) => unavailable(request),
        _ => reply(
            request,
            StatusCode::CONFLICT,
            "state_conflict",
            "job state does not allow this action",
        ),
    }
}

fn bad(request: RequestId) -> Response {
    reply(
        request,
        StatusCode::BAD_REQUEST,
        "bad_request",
        "invalid job parameters",
    )
}

fn unavailable(request: RequestId) -> Response {
    reply(
        request,
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_unavailable",
        "job queue unavailable",
    )
}

fn scan_pending(request: RequestId) -> Response {
    reply(
        request,
        StatusCode::CONFLICT,
        "scan_pending",
        "a file scan is already pending",
    )
}

fn reply(request: RequestId, status: StatusCode, code: &str, message: &str) -> Response {
    ApiErrorBody::new(code, message, request.get()).into_response(status)
}
