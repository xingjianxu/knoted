//! Explicit metadata transfer between stable file sources.
use crate::{
    error::{ApiErrorBody, FieldError},
    middleware::{
        RequestId,
        auth::{require_csrf, require_session},
    },
};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{StatusCode, header},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::post,
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    files::identity::{self, DomainError, RelinkSource},
};
use serde::Deserialize;
use uuid::Uuid;
pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/files/{id}/relink", post(relink))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    new_source_id: Uuid,
    expected_version: i64,
}
async fn relink(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<Body>, JsonRejection>,
) -> Response {
    let (Ok(Path(id)), Ok(Json(body))) = (path, body) else {
        return ApiErrorBody::new("bad_request", "invalid request parameters", request.get())
            .into_response(StatusCode::BAD_REQUEST);
    };
    match identity::relink_source(
        &ctx,
        RelinkSource {
            id,
            new_source_id: body.new_source_id,
            expected_version: body.expected_version,
        },
    )
    .await
    {
        Ok(source) => (
            [(header::ETAG, format!("\"{}\"", source.version))],
            Json(source),
        )
            .into_response(),
        Err(error) => {
            let (status, code, message) = match &error {
                DomainError::NotFound => {
                    (StatusCode::NOT_FOUND, "not_found", "file source not found")
                }
                DomainError::Conflict { .. } => (
                    StatusCode::CONFLICT,
                    "version_conflict",
                    "version or state conflict",
                ),
                DomainError::InvalidField => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_failed",
                    "invalid relink pair or version",
                ),
                DomainError::InvalidQuery => {
                    (StatusCode::BAD_REQUEST, "bad_request", "invalid request")
                }
                DomainError::Database(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "dependency_unavailable",
                    "file persistence unavailable",
                ),
            };
            let mut result = ApiErrorBody::new(code, message, request.get());
            if let DomainError::Conflict {
                current_version: Some(version),
            } = error
            {
                result = result.with_current_version(version);
            }
            if matches!(error, DomainError::InvalidField) {
                result = result.with_field_errors(vec![FieldError {
                    field: if body.expected_version < 1 {
                        "expected_version"
                    } else {
                        "new_source_id"
                    }
                    .into(),
                    message: "must specify a distinct source and positive metadata version".into(),
                }]);
            }
            result.into_response(status)
        }
    }
}
