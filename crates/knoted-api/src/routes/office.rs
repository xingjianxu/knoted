//! Browser preview writes and a separate capability-only source route.
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query, RawQuery, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{StatusCode, header},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    files::office::{self, CreatePreview, DomainError, GetPreviewSource, RevokePreview},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    error::ApiErrorBody,
    middleware::{
        RequestId,
        auth::{CurrentSession, require_csrf, require_session},
    },
};

const NO_STORE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "private, no-store");

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/files/{id}/office-preview", post(create).delete(revoke))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
        .route(
            "/office-preview-sources/{id}",
            get(source).head(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    expected_content_version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeQuery {
    preview_id: Uuid,
}

async fn create(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let (Ok(Path(file_id)), Ok(Json(body))) = (path, body) else {
        return bad_request(request);
    };
    match office::create_preview(
        &ctx,
        CreatePreview {
            file_id,
            expected_content_version: body.expected_content_version,
            session_id: current.session.id,
        },
    )
    .await
    {
        Ok(preview) => (StatusCode::CREATED, [NO_STORE], Json(preview)).into_response(),
        Err(error) => {
            let (status, code) = match error {
                DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
                DomainError::Conflict => (StatusCode::CONFLICT, "version_conflict"),
                DomainError::Unsupported => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
                DomainError::AtCapacity => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests"),
                DomainError::Unavailable => {
                    (StatusCode::SERVICE_UNAVAILABLE, "dependency_unavailable")
                }
                DomainError::SessionEnded => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            };
            ApiErrorBody::new(code, error.to_string(), request.get()).into_response(status)
        }
    }
}

async fn revoke(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<RevokeQuery>, QueryRejection>,
) -> Response {
    let (Ok(Path(file_id)), Ok(Query(query))) = (path, query) else {
        return bad_request(request);
    };
    office::revoke_preview(
        &ctx,
        RevokePreview {
            file_id,
            preview_id: query.preview_id,
            session_id: current.session.id,
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn source(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
) -> Response {
    // Avoid QueryRejection: its diagnostic can echo an invalid query value.
    // Our capability has one canonical ASCII representation; reject extras.
    if let (Ok(Path(id)), Some(token)) = (
        path,
        query.as_deref().and_then(|q| q.strip_prefix("token=")),
    ) && let Ok(preview_id) = id.parse()
        && let Some(source) =
            office::get_preview_source(&ctx, GetPreviewSource { preview_id, token }).await
    {
        return (
            [
                NO_STORE,
                (header::CONTENT_TYPE, source.mime),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            source.bytes,
        )
            .into_response();
    }
    let mut response = ApiErrorBody::new("not_found", "preview source not found", request.get())
        .into_response(StatusCode::NOT_FOUND);
    response
        .headers_mut()
        .insert(NO_STORE.0, header::HeaderValue::from_static(NO_STORE.1));
    response
}

fn bad_request(request: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", request.get())
        .into_response(StatusCode::BAD_REQUEST)
}
