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
        Path, Query, State,
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
    files::{
        browse,
        identity::{self, DomainError, RelinkSource},
    },
};
use serde::Deserialize;
use uuid::Uuid;
pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/files", get(list))
        .route("/files/{id}", get(detail).patch(update))
        .route("/files/{id}/content", get(content))
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<u32>,
    cursor: Option<String>,
}
fn invalid(request: &RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", request.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn file_error(error: DomainError, request: &RequestId) -> Response {
    let (status, code) = match &error {
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        DomainError::Conflict { .. } => (StatusCode::CONFLICT, "version_conflict"),
        DomainError::InvalidField => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
        DomainError::InvalidQuery => (StatusCode::BAD_REQUEST, "bad_request"),
        DomainError::Database(_) => (StatusCode::SERVICE_UNAVAILABLE, "dependency_unavailable"),
    };
    let mut body = ApiErrorBody::new(code, "file operation failed", request.get());
    if let DomainError::Conflict {
        current_version: Some(v),
    } = error
    {
        body = body.with_current_version(v);
    }
    if matches!(error, DomainError::InvalidField) {
        body = body.with_field_errors(vec![FieldError {
            field: "note/tag_ids/expected_version".into(),
            message: "invalid metadata update".into(),
        }]);
    }
    body.into_response(status)
}
fn file_response(source: knoted_core::files::identity::FileSource) -> Response {
    (
        [(header::ETAG, format!("\"{}\"", source.version))],
        Json(source),
    )
        .into_response()
}
async fn list(
    State(ctx): State<AppContext>,
    request: RequestId,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(q)) = query else {
        return invalid(&request);
    };
    match browse::list_files(
        &ctx,
        browse::ListFiles {
            limit: q.limit,
            cursor: q.cursor,
        },
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => file_error(e, &request),
    }
}
async fn detail(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return invalid(&request);
    };
    match browse::get_file(&ctx, browse::GetFile { id }).await {
        Ok(s) => file_response(s),
        Err(e) => file_error(e, &request),
    }
}
fn nullable_note<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}
fn tags<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<Uuid>>, D::Error> {
    Vec::<Uuid>::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    expected_version: i64,
    #[serde(default, deserialize_with = "nullable_note")]
    note: Option<Option<String>>,
    #[serde(default, deserialize_with = "tags")]
    tag_ids: Option<Vec<Uuid>>,
}
async fn update(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let (Ok(Path(id)), Ok(Json(b))) = (path, body) else {
        return invalid(&request);
    };
    match browse::update_file(
        &ctx,
        browse::UpdateFile {
            id,
            expected_version: b.expected_version,
            note: b.note,
            tag_ids: b.tag_ids,
        },
    )
    .await
    {
        Ok(s) => file_response(s),
        Err(e) => file_error(e, &request),
    }
}
async fn content(
    State(ctx): State<AppContext>,
    request: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return invalid(&request);
    };
    use knoted_core::files::download::{self, DomainError as ContentError};
    match download::read_content(&ctx, download::ReadContent { id }).await {
        Ok(content) => {
            let length = content.bytes.len();
            (
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                    (header::CONTENT_LENGTH, length.to_string()),
                    (header::CONTENT_DISPOSITION, attachment(&content.name)),
                    (header::ETAG, format!("\"{}\"", content.version)),
                    (header::CACHE_CONTROL, "private, no-store".into()),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                ],
                content.bytes,
            )
                .into_response()
        }
        Err(error) => {
            let (status, code) = if matches!(error, ContentError::NotFound) {
                (StatusCode::NOT_FOUND, "not_found")
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, "dependency_unavailable")
            };
            ApiErrorBody::new(code, "file content unavailable", request.get()).into_response(status)
        }
    }
}
fn attachment(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .collect();
    let name = if name.is_empty() { "download" } else { &name };
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = name
        .as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn t49_attachment_header_is_safe_and_unicode_roundtrippable() {
        let value = super::attachment("中\"\r\n/\\文.txt");
        assert_eq!(
            value,
            "attachment; filename=\"___.txt\"; filename*=UTF-8''%E4%B8%AD%22%E6%96%87.txt"
        );
        assert!(axum::http::HeaderValue::from_str(&value).is_ok());
        assert!(super::attachment("").contains("filename=\"download\""));
    }
}
