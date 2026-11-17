//! Task follow-up HTTP DTOs; core owns validation, CAS and transactions.
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
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::get,
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    tasks::{
        activities::{
            self, Activity, AppendActivity, DeleteActivity, EditActivity, ListActivities,
            ListRevisions,
        },
        service::DomainError,
    },
};
use serde::Deserialize;
use uuid::Uuid;

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/tasks/{task_id}/activities", get(list).post(append))
        .route(
            "/tasks/{task_id}/activities/{activity_id}",
            axum::routing::patch(edit).delete(delete),
        )
        .route(
            "/tasks/{task_id}/activities/{activity_id}/revisions",
            get(revisions),
        )
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendBody {
    body: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditBody {
    expected_version: i64,
    body: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionQuery {
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn error(id: RequestId, e: DomainError) -> Response {
    let (status, code, message) = match &e {
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found", "activity not found"),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "activity version or state conflict",
        ),
        DomainError::InvalidQuery => (
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid activity list parameters",
        ),
        DomainError::InvalidField(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid activity field",
        ),
        DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "activity persistence unavailable",
        ),
    };
    let mut b = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(v),
    } = e
    {
        b = b.with_current_version(v);
    }
    b.into_response(status)
}
fn created(a: Activity) -> Response {
    (StatusCode::CREATED, Json(a)).into_response()
}
async fn list(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let (Ok(Path(task_id)), Ok(Query(q))) = (path, query) else {
        return bad(id);
    };
    match activities::list(
        &ctx,
        ListActivities {
            task_id,
            cursor: q.cursor,
            limit: q.limit,
        },
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => error(id, e),
    }
}
async fn append(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<AppendBody>, JsonRejection>,
) -> Response {
    let (Ok(Path(task_id)), Ok(Json(b))) = (path, body) else {
        return bad(id);
    };
    match activities::append(
        &ctx,
        AppendActivity {
            task_id,
            body: b.body,
        },
    )
    .await
    {
        Ok(a) => created(a),
        Err(e) => error(id, e),
    }
}
async fn edit(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    body: Result<Json<EditBody>, JsonRejection>,
) -> Response {
    let (Ok(Path((task_id, activity_id))), Ok(Json(b))) = (path, body) else {
        return bad(id);
    };
    match activities::edit(
        &ctx,
        EditActivity {
            task_id,
            id: activity_id,
            expected_version: b.expected_version,
            body: b.body,
        },
    )
    .await
    {
        Ok(a) => Json(a).into_response(),
        Err(e) => error(id, e),
    }
}
async fn delete(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    query: Result<Query<VersionQuery>, QueryRejection>,
) -> Response {
    let (Ok(Path((task_id, activity_id))), Ok(Query(q))) = (path, query) else {
        return bad(id);
    };
    match activities::delete(
        &ctx,
        DeleteActivity {
            task_id,
            id: activity_id,
            expected_version: q.expected_version,
        },
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(id, e),
    }
}
async fn revisions(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let (Ok(Path((task_id, activity_id))), Ok(Query(q))) = (path, query) else {
        return bad(id);
    };
    match activities::list_revisions(
        &ctx,
        ListRevisions {
            task_id,
            activity_id,
            cursor: q.cursor,
            limit: q.limit,
        },
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(e) => error(id, e),
    }
}
