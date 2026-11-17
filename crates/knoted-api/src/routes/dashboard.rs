//! Authenticated read-only dashboard; domain/database diagnostics stay private.
use crate::{
    error::ApiErrorBody,
    middleware::{RequestId, auth::require_session},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::get,
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    dashboard::{self, GetDashboard},
};
pub fn router(_config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/dashboard", get(summary))
        .route_layer(from_fn_with_state(ctx, require_session))
}
async fn summary(State(ctx): State<AppContext>, id: RequestId) -> Response {
    match dashboard::summary(&ctx, GetDashboard).await {
        Ok(value) => Json(value).into_response(),
        Err(_) => ApiErrorBody::new("dependency_unavailable", "dashboard unavailable", id.get())
            .into_response(StatusCode::SERVICE_UNAVAILABLE),
    }
}
