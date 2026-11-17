//! HTTP route assembly.

pub mod assets;
pub mod auth;
pub mod health;
pub mod notes;
pub mod sessions;

use axum::Router;
use axum::http::StatusCode;
use axum::response::Response;
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;

/// The application router, without logging or the request id layer.
///
/// Callers that need those layers (the binary and [`crate::build_router`]) wrap
/// this router with [`crate::with_transport_layers`]; tests may add routes
/// before wrapping.
pub fn router(ctx: AppContext, auth: AuthConfig) -> Router {
    health::router()
        .nest(
            "/api/v1",
            auth::router(auth.clone(), ctx.clone())
                .merge(sessions::router(auth.clone(), ctx.clone()))
                .merge(notes::router(auth.clone(), ctx.clone()))
                .merge(assets::router(auth, ctx.clone())),
        )
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(ctx)
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

/// A known path with an unsupported method answers in the same error shape, not
/// Axum's empty 405.
async fn method_not_allowed(request_id: RequestId) -> Response {
    ApiErrorBody::new(
        "method_not_allowed",
        "this method is not supported for the requested path",
        request_id.get(),
    )
    .into_response(StatusCode::METHOD_NOT_ALLOWED)
}
