//! Asset transport; core owns decoding, storage and transactional registration.
use crate::{
    error::ApiErrorBody,
    middleware::{
        RequestId,
        auth::{CurrentSession, require_csrf, require_session},
    },
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Multipart, Path, State, multipart::MultipartRejection,
        rejection::PathRejection,
    },
    http::{HeaderMap, StatusCode, header},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use knoted_core::{
    assets::service::{self, DomainError, MAX_BYTES, ReadAsset, UploadAsset},
    config::AuthConfig,
    context::AppContext,
};
use uuid::Uuid;

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/assets", post(upload))
        .route("/assets/{asset_id}/content", get(read))
        .layer(DefaultBodyLimit::max(MAX_BYTES + 64 * 1024))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid asset request", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn error(id: RequestId, error: DomainError) -> Response {
    let (status, code, message) = match error {
        DomainError::Limit => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "limit_exceeded",
            "image limit exceeded",
        ),
        DomainError::InvalidImage | DomainError::InvalidReference => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid image contents or digest",
        ),
        DomainError::InvalidKey => return bad(id),
        DomainError::Conflict => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "idempotency key request conflict",
        ),
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found", "asset not found"),
        DomainError::Storage | DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "asset storage unavailable",
        ),
    };
    ApiErrorBody::new(code, message, id.get()).into_response(status)
}
fn multipart_error(id: RequestId, status: StatusCode) -> Response {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        error(id, DomainError::Limit)
    } else {
        bad(id)
    }
}
async fn upload(
    State(ctx): State<AppContext>,
    Extension(session): Extension<CurrentSession>,
    id: RequestId,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let values = headers.get_all("idempotency-key");
    let mut values = values.iter();
    let key = match (values.next().and_then(|h| h.to_str().ok()), values.next()) {
        (Some(k), None) if !k.is_empty() && k.len() <= 255 => k.to_owned(),
        _ => return bad(id),
    };
    let mut multipart = match multipart {
        Ok(v) => v,
        Err(e) => return multipart_error(id, e.status()),
    };
    let mut bytes = None;
    let mut hash = None;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return multipart_error(id, e.status()),
        };
        let name = field.name().unwrap_or("").to_owned();
        let limit = match name.as_str() {
            "file" if bytes.is_none() => MAX_BYTES,
            "sha256" if hash.is_none() => 64,
            _ => return bad(id),
        };
        let mut value = Vec::new();
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if value.len() + chunk.len() > limit {
                        return if name == "file" {
                            error(id, DomainError::Limit)
                        } else {
                            bad(id)
                        };
                    }
                    value.extend_from_slice(&chunk)
                }
                Ok(None) => break,
                Err(e) => return multipart_error(id, e.status()),
            }
        }
        match name.as_str() {
            "file" => bytes = Some(value),
            "sha256" => {
                let Ok(s) = String::from_utf8(value) else {
                    return bad(id);
                };
                if s.len() != 64
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return bad(id);
                }
                hash = Some(s);
            }
            _ => return bad(id),
        }
    }
    let Some(bytes) = bytes else { return bad(id) };
    match service::upload_admitted(
        &ctx,
        UploadAsset {
            owner_id: session.session.user_id,
            idempotency_key: key,
            bytes,
            sha256: hash,
        },
    )
    .await
    {
        Ok(asset) => (StatusCode::CREATED, Json(asset)).into_response(),
        Err(e) => error(id, e),
    }
}
async fn read(
    State(ctx): State<AppContext>,
    Extension(session): Extension<CurrentSession>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(asset_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::read(
        &ctx,
        ReadAsset {
            id: asset_id,
            owner_id: session.session.user_id,
        },
    )
    .await
    {
        Ok(content) => {
            let mut response = Body::from(content.bytes).into_response();
            let h = response.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                content.asset.mime.parse().expect("stored supported MIME"),
            );
            h.insert(
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{}\"", content.asset.id)
                    .parse()
                    .expect("UUID header"),
            );
            h.insert(
                header::ETAG,
                format!("\"{}\"", content.asset.sha256)
                    .parse()
                    .expect("digest header"),
            );
            h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
            h.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
            response
        }
        Err(e) => error(id, e),
    }
}
