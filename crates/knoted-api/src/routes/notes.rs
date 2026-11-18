//! HTTP adaptation for T03 notes. Persistence and validation live in core.
use crate::{
    error::ApiErrorBody,
    middleware::{
        RequestId,
        auth::{require_csrf, require_session},
    },
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
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
    notes::{
        history::{self, FlushSnapshot, ListSnapshots, RestoreSnapshot},
        service::{
            self, CreateNote, DeleteNote, DomainError, GetNote, ListNotes, Note, UpdateNote,
        },
        tree::{self, MoveNote, RestoreNote, RestoreParent},
    },
};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use uuid::Uuid;

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/notes", get(list).post(create))
        .route(
            "/notes/{note_id}",
            get(get_note).patch(update).delete(delete),
        )
        .route("/notes/{note_id}/move", post(move_note))
        .route("/notes/{note_id}/restore", post(restore))
        .route(
            "/notes/{note_id}/snapshots",
            get(list_snapshots).post(flush_snapshot),
        )
        .route(
            "/notes/{note_id}/snapshots/{snapshot_id}/restore",
            post(restore_snapshot),
        )
        // Enough for a 2MiB raw document plus the bounded title and envelope.
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 + 64 * 1024))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    title: String,
    parent_id: Option<Uuid>,
    document: Box<RawValue>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    expected_version: i64,
    #[serde(default, deserialize_with = "present")]
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    document: Option<Box<RawValue>>,
}
// Missing means unchanged; explicit null is invalid for mutable content fields.
fn present<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<T>, D::Error> {
    if let Some(value) = Option::<T>::deserialize(d)? {
        Ok(Some(value))
    } else {
        Err(serde::de::Error::custom("null is not allowed"))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    parent_id: Option<Uuid>,
    limit: Option<u32>,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteQuery {
    expected_version: i64,
}

fn response(note: Note, status: StatusCode) -> Response {
    let etag = format!("\"{}\"", note.content_version);
    (status, [(header::ETAG, etag)], Json(note)).into_response()
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn json_error(id: RequestId, error: JsonRejection) -> Response {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiErrorBody::new("limit_exceeded", "note request is too large", id.get())
            .into_response(StatusCode::PAYLOAD_TOO_LARGE)
    } else {
        bad(id)
    }
}
pub(super) fn error(id: RequestId, error: DomainError) -> Response {
    let (status, code, message) = match &error {
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found", "note not found"),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "note version or state conflict",
        ),
        DomainError::InvalidQuery => (
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid note list parameters",
        ),
        DomainError::InvalidField(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid note field",
        ),
        DomainError::Document(e) if e.status_code() == 413 => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "limit_exceeded",
            "note document limit exceeded",
        ),
        DomainError::Document(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid note document",
        ),
        DomainError::Asset(knoted_core::assets::service::DomainError::InvalidReference) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "unknown image asset reference",
        ),
        DomainError::Asset(_) | DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "note persistence unavailable",
        ),
    };
    let mut body = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(v),
    } = error
    {
        body = body.with_current_version(v);
    }
    body.into_response(status)
}
async fn create(
    State(ctx): State<AppContext>,
    id: RequestId,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(v) => v,
        Err(e) => return json_error(id, e),
    };
    match service::create(
        &ctx,
        CreateNote {
            title: body.title,
            parent_id: body.parent_id,
            document_json: body.document.get().to_owned(),
        },
    )
    .await
    {
        Ok(note) => response(note, StatusCode::CREATED),
        Err(e) => error(id, e),
    }
}
async fn get_note(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::get(&ctx, GetNote { id: note_id }).await {
        Ok(note) => response(note, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn list(
    State(ctx): State<AppContext>,
    id: RequestId,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::list(
        &ctx,
        ListNotes {
            parent_id: q.parent_id,
            limit: q.limit,
            cursor: q.cursor,
        },
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => error(id, e),
    }
}
async fn update(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(body) = match body {
        Ok(v) => v,
        Err(e) => return json_error(id, e),
    };
    match service::update(
        &ctx,
        UpdateNote {
            id: note_id,
            expected_version: body.expected_version,
            title: body.title,
            document_json: body.document.map(|d| d.get().to_owned()),
        },
    )
    .await
    {
        Ok(note) => response(note, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn delete(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<DeleteQuery>, QueryRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Query(q) = match query {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::delete(
        &ctx,
        DeleteNote {
            id: note_id,
            expected_version: q.expected_version,
        },
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveBody {
    // The deserializer makes the nullable field required, as in T03.
    #[serde(deserialize_with = "required_parent")]
    parent_id: Option<Uuid>,
    position: i64,
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreParentBody {
    #[serde(deserialize_with = "required_parent")]
    parent_id: Option<Uuid>,
}
fn required_parent<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(d)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreBody {
    expected_version: i64,
    #[serde(default, deserialize_with = "present")]
    restore_parent: Option<RestoreParentBody>,
}
async fn move_note(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<MoveBody>, JsonRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(body) = match body {
        Ok(v) => v,
        Err(e) => return json_error(id, e),
    };
    match tree::move_note(
        &ctx,
        MoveNote {
            id: note_id,
            parent_id: body.parent_id,
            position: body.position,
            expected_version: body.expected_version,
        },
    )
    .await
    {
        Ok(note) => response(note, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn restore(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<RestoreBody>, JsonRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(body) = match body {
        Ok(v) => v,
        Err(e) => return json_error(id, e),
    };
    match tree::restore(
        &ctx,
        RestoreNote {
            id: note_id,
            expected_version: body.expected_version,
            restore_parent: body.restore_parent.map(|p| RestoreParent {
                parent_id: p.parent_id,
            }),
        },
    )
    .await
    {
        Ok(note) => response(note, StatusCode::OK),
        Err(e) => error(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotQuery {
    limit: Option<u32>,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRestoreBody {
    expected_version: i64,
}
async fn flush_snapshot(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(p) => p,
        Err(_) => return bad(id),
    };
    match history::flush_snapshot(&ctx, FlushSnapshot { note_id }).await {
        Ok(s) => (StatusCode::CREATED, Json(s)).into_response(),
        Err(e) => error(id, e),
    }
}
async fn list_snapshots(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<SnapshotQuery>, QueryRejection>,
) -> Response {
    let Path(note_id) = match path {
        Ok(p) => p,
        Err(_) => return bad(id),
    };
    let Query(q) = match query {
        Ok(q) => q,
        Err(_) => return bad(id),
    };
    match history::list_snapshots(
        &ctx,
        ListSnapshots {
            note_id,
            limit: q.limit,
            cursor: q.cursor,
        },
    )
    .await
    {
        Ok(p) => Json(p).into_response(),
        Err(e) => error(id, e),
    }
}
async fn restore_snapshot(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    body: Result<Json<SnapshotRestoreBody>, JsonRejection>,
) -> Response {
    let Path((note_id, snapshot_id)) = match path {
        Ok(p) => p,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(b) => b,
        Err(e) => return json_error(id, e),
    };
    match history::restore_snapshot(
        &ctx,
        RestoreSnapshot {
            note_id,
            snapshot_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(n) => response(n, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
