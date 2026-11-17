//! Task HTTP DTO presence semantics; core owns validation and transactions.
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
    tasks::{
        lifecycle::{self, ArchiveTask, RestoreTask, UnarchiveTask},
        service::{
            self, CreateTask, DateValue, DeleteTask, DomainError, GetTask, ListTasks, Task,
            TaskPriority, TaskStatus, UpdateTask,
        },
    },
};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;
pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/tasks", get(list).post(create))
        .route(
            "/tasks/{task_id}",
            get(get_task).patch(update).delete(delete),
        )
        .route("/tasks/{task_id}/archive", post(archive))
        .route("/tasks/{task_id}/unarchive", post(unarchive))
        .route("/tasks/{task_id}/restore", post(restore))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
fn non_null<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    title: String,
    description: Option<String>,
    parent_id: Option<Uuid>,
    #[serde(default, deserialize_with = "nullable")]
    project_id: Option<Option<Uuid>>,
    #[serde(default)]
    status: TaskStatus,
    #[serde(default)]
    priority: TaskPriority,
    start: Option<DateValue>,
    due: Option<DateValue>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    expected_version: i64,
    #[serde(default, deserialize_with = "non_null")]
    title: Option<String>,
    #[serde(default, deserialize_with = "non_null")]
    status: Option<TaskStatus>,
    #[serde(default, deserialize_with = "non_null")]
    priority: Option<TaskPriority>,
    #[serde(default, deserialize_with = "nullable")]
    project_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "nullable")]
    parent_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "nullable")]
    description: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    start: Option<Option<DateValue>>,
    #[serde(default, deserialize_with = "nullable")]
    due: Option<Option<DateValue>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionQuery {
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    project_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    status: Option<TaskStatus>,
    priority: Option<TaskPriority>,
    archived: Option<bool>,
}
fn response(t: Task, status: StatusCode) -> Response {
    let etag = format!("\"{}\"", t.version);
    (status, [(header::ETAG, etag)], Json(t)).into_response()
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn error(id: RequestId, e: DomainError) -> Response {
    let (status, code, message) = match &e {
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found", "task not found"),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "task version or state conflict",
        ),
        DomainError::InvalidQuery => (
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid task list parameters",
        ),
        DomainError::InvalidField(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid task field",
        ),
        DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "task persistence unavailable",
        ),
    };
    let mut b = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(v),
    } = e
    {
        b = b.with_current_version(v);
    }
    if let DomainError::InvalidField(field) = e {
        b = b.with_field_errors(vec![FieldError {
            field: field.into(),
            message: "invalid task field".into(),
        }]);
    }
    b.into_response(status)
}
async fn create(
    State(ctx): State<AppContext>,
    id: RequestId,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::create(
        &ctx,
        CreateTask {
            title: b.title,
            description: b.description,
            parent_id: b.parent_id,
            project_id: b.project_id,
            status: b.status,
            priority: b.priority,
            start: b.start,
            due: b.due,
        },
    )
    .await
    {
        Ok(t) => response(t, StatusCode::CREATED),
        Err(e) => error(id, e),
    }
}
async fn get_task(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::get(&ctx, GetTask { id: task_id }).await {
        Ok(t) => response(t, StatusCode::OK),
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
        ListTasks {
            cursor: q.cursor,
            limit: q.limit,
            project_id: q.project_id,
            parent_id: q.parent_id,
            status: q.status,
            priority: q.priority,
            archived: q.archived,
        },
    )
    .await
    {
        Ok(t) => Json(t).into_response(),
        Err(e) => error(id, e),
    }
}
async fn update(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::update(
        &ctx,
        UpdateTask {
            id: task_id,
            expected_version: b.expected_version,
            title: b.title,
            description: b.description,
            parent_id: b.parent_id,
            project_id: b.project_id,
            status: b.status,
            priority: b.priority,
            start: b.start,
            due: b.due,
        },
    )
    .await
    {
        Ok(t) => response(t, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn delete(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<VersionQuery>, QueryRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Query(q) = match query {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::delete(
        &ctx,
        DeleteTask {
            id: task_id,
            expected_version: q.expected_version,
        },
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(id, e),
    }
}

async fn archive(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<VersionQuery>, JsonRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match lifecycle::archive(
        &ctx,
        ArchiveTask {
            id: task_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(t) => response(t, StatusCode::OK),
        Err(e) => error(id, e),
    }
}

async fn restore(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<VersionQuery>, JsonRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match lifecycle::restore(
        &ctx,
        RestoreTask {
            id: task_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(t) => response(t, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn unarchive(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<VersionQuery>, JsonRejection>,
) -> Response {
    let Path(task_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match lifecycle::unarchive(
        &ctx,
        UnarchiveTask {
            id: task_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(t) => response(t, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
