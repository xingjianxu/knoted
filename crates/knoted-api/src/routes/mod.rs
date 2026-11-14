//! HTTP route assembly.

pub mod health;

use axum::Router;
use axum::http::StatusCode;
use axum::response::Response;
use knoted_core::context::AppContext;

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;

/// The application router, without logging or the request id layer.
///
/// Callers that need those layers (the binary and [`crate::build_router`]) wrap
/// this router; tests may add routes before wrapping.
pub fn router(ctx: AppContext) -> Router {
    health::router().fallback(not_found).with_state(ctx)
}

/// Unknown paths answer in the shared error shape, not Axum's empty 404.
async fn not_found(request_id: RequestId) -> Response {
    ApiErrorBody::new(
        "not_found",
        "no route matches this request",
        request_id.get(),
    )
    .into_response(StatusCode::NOT_FOUND)
}
