//! Process health endpoints.
//!
//! These are operational probes, not product API: they are mounted at the root
//! (`/health/...`, never `/api/v1`) and are intentionally absent from
//! `api/openapi.yaml`.
//!
//! Liveness answers "is this process running" and touches nothing else.
//! Readiness answers "can this process serve product traffic" and reuses
//! [`knoted_core::db::is_ready`], which treats a missing, failed, unknown or
//! checksum-drifted migration as not ready. An optional dependency such as a
//! model runtime must never turn into a liveness failure.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use knoted_core::context::AppContext;
use serde::Serialize;

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;

/// Stable error code for a dependency that cannot serve traffic.
const DEPENDENCY_UNAVAILABLE: &str = "dependency_unavailable";

/// Health responses.
#[derive(Debug, Serialize)]
struct StatusBody {
    status: &'static str,
}

/// Health routes, to be mounted by the application router.
pub fn router() -> Router<AppContext> {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
}

/// The process is up; the response does not depend on the database or models.
async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(StatusBody { status: "live" }))
}

/// The process can serve traffic: the schema matches the embedded migrations.
async fn ready(State(ctx): State<AppContext>, request_id: RequestId) -> Response {
    match knoted_core::db::is_ready(ctx.pool()).await {
        Ok(true) => (StatusCode::OK, Json(StatusBody { status: "ready" })).into_response(),
        Ok(false) => unavailable(
            "the database schema is not ready for this build",
            request_id,
        ),
        Err(error) => {
            tracing::warn!(
                target: "knoted_api::health",
                request_id = %request_id.get(),
                error = %error,
                "readiness check failed"
            );
            unavailable("the database readiness check failed", request_id)
        }
    }
}

/// A 503 in the shared error shape.
fn unavailable(message: &'static str, request_id: RequestId) -> Response {
    ApiErrorBody::new(DEPENDENCY_UNAVAILABLE, message, request_id.get())
        .into_response(StatusCode::SERVICE_UNAVAILABLE)
}
