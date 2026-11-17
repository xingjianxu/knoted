//! Project HTTP contract; lifecycle transactions and validation belong to core.
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
    http::{StatusCode, header},
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    projects::service::{
        self, ArchiveProject, CreateProject, DeleteProject, DomainError, GetProject, ListProjects,
        Project, RestoreProject, UnarchiveProject, UpdateProject,
    },
};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/projects", get(list).post(create))
        .route(
            "/projects/{project_id}",
            get(get_project).patch(update).delete(delete),
        )
        .route("/projects/{project_id}/archive", post(archive))
        .route("/projects/{project_id}/unarchive", post(unarchive))
        .route("/projects/{project_id}/restore", post(restore))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    title: String,
    description: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    expected_version: i64,
    #[serde(default, deserialize_with = "non_null")]
    title: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    description: Option<Option<String>>,
}
fn non_null<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionBody {
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    archived: Option<bool>,
}
fn response(p: Project, status: StatusCode) -> Response {
    let etag = format!("\"{}\"", p.version);
    (status, [(header::ETAG, etag)], Json(p)).into_response()
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn error(id: RequestId, e: DomainError) -> Response {
    let (status, code, message) = match &e {
        DomainError::NotFound => (StatusCode::NOT_FOUND, "not_found", "project not found"),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "project version or state conflict",
        ),
        DomainError::InvalidQuery => (
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid project list parameters",
        ),
        DomainError::InvalidField(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid project field",
        ),
        DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "project persistence unavailable",
        ),
    };
    let mut body = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(v),
    } = e
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
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::create(
        &ctx,
        CreateProject {
            title: b.title,
            description: b.description,
        },
    )
    .await
    {
        Ok(p) => response(p, StatusCode::CREATED),
        Err(e) => error(id, e),
    }
}
async fn get_project(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::get(&ctx, GetProject { id: project_id }).await {
        Ok(p) => response(p, StatusCode::OK),
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
        ListProjects {
            cursor: q.cursor,
            limit: q.limit,
            archived: q.archived,
        },
    )
    .await
    {
        Ok(p) => Json(p).into_response(),
        Err(e) => error(id, e),
    }
}
async fn update(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::update(
        &ctx,
        UpdateProject {
            id: project_id,
            expected_version: b.expected_version,
            title: b.title,
            description: b.description,
        },
    )
    .await
    {
        Ok(p) => response(p, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn delete(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<VersionBody>, QueryRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Query(b) = match query {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::delete(
        &ctx,
        DeleteProject {
            id: project_id,
            expected_version: b.expected_version,
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
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::archive(
        &ctx,
        ArchiveProject {
            id: project_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(p) => response(p, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn unarchive(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::unarchive(
        &ctx,
        UnarchiveProject {
            id: project_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(p) => response(p, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn restore(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<VersionBody>, JsonRejection>,
) -> Response {
    let Path(project_id) = match path {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    let Json(b) = match body {
        Ok(v) => v,
        Err(_) => return bad(id),
    };
    match service::restore(
        &ctx,
        RestoreProject {
            id: project_id,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(p) => response(p, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
