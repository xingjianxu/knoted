//! Transport adaptation for tags and stable entity associations.
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
    routing::{delete, get, put},
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    links::service::{self, *},
};
use serde::Deserialize;
use uuid::Uuid;

pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/tags", get(list_tags).post(create_tag))
        .route(
            "/tags/{id}",
            axum::routing::patch(update_tag).delete(delete_tag),
        )
        .route("/links", get(list_links).post(create_link))
        .route("/links/{id}", delete(remove_link))
        .route("/entities/{kind}/{id}/tags", get(entity_tags))
        .route(
            "/entities/{kind}/{id}/tags/{tag}",
            put(assign_tag).delete(remove_tag),
        )
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameBody {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    name: String,
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    expected_version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Paging {
    limit: Option<u32>,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkQuery {
    source_type: EntityType,
    source_id: Uuid,
    limit: Option<u32>,
    cursor: Option<String>,
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid request parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
fn error(id: RequestId, e: DomainError) -> Response {
    let (status, code, message) = match &e {
        DomainError::NotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "entity or tag not found",
        ),
        DomainError::Conflict { .. } => (
            StatusCode::CONFLICT,
            "version_conflict",
            "version or state conflict",
        ),
        DomainError::InvalidField => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "invalid tag name",
        ),
        DomainError::InvalidQuery => (StatusCode::BAD_REQUEST, "bad_request", "invalid pagination"),
        DomainError::Database(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "association persistence unavailable",
        ),
    };
    let mut body = ApiErrorBody::new(code, message, id.get());
    if let DomainError::Conflict {
        current_version: Some(version),
    } = e
    {
        body = body.with_current_version(version);
    }
    body.into_response(status)
}
fn tag_response(t: Tag, status: StatusCode) -> Response {
    (
        status,
        [(header::ETAG, format!("\"{}\"", t.version))],
        Json(t),
    )
        .into_response()
}
async fn create_tag(
    State(ctx): State<AppContext>,
    id: RequestId,
    body: Result<Json<NameBody>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else { return bad(id) };
    match service::create_tag(&ctx, CreateTag { name: b.name }).await {
        Ok(t) => tag_response(t, StatusCode::CREATED),
        Err(e) => error(id, e),
    }
}
async fn update_tag(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    body: Result<Json<UpdateBody>, JsonRejection>,
) -> Response {
    let (Ok(Path(tag_id)), Ok(Json(b))) = (path, body) else {
        return bad(id);
    };
    match service::update_tag(
        &ctx,
        UpdateTag {
            id: tag_id,
            name: b.name,
            expected_version: b.expected_version,
        },
    )
    .await
    {
        Ok(t) => tag_response(t, StatusCode::OK),
        Err(e) => error(id, e),
    }
}
async fn delete_tag(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    q: Result<Query<Version>, QueryRejection>,
) -> Response {
    let (Ok(Path(tag_id)), Ok(Query(q))) = (path, q) else {
        return bad(id);
    };
    unit(
        id,
        service::delete_tag(
            &ctx,
            DeleteTag {
                id: tag_id,
                expected_version: q.expected_version,
            },
        )
        .await,
    )
}
async fn list_tags(
    State(ctx): State<AppContext>,
    id: RequestId,
    q: Result<Query<Paging>, QueryRejection>,
) -> Response {
    let Ok(Query(q)) = q else { return bad(id) };
    match service::list_tags(
        &ctx,
        ListTags {
            entity: None,
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
async fn entity_tags(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(EntityType, Uuid)>, PathRejection>,
    q: Result<Query<Paging>, QueryRejection>,
) -> Response {
    let (Ok(Path((kind, entity_id))), Ok(Query(q))) = (path, q) else {
        return bad(id);
    };
    match service::list_tags(
        &ctx,
        ListTags {
            entity: Some(Entity {
                entity_type: kind,
                id: entity_id,
            }),
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
fn unit(id: RequestId, result: Result<(), DomainError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(id, e),
    }
}
async fn assign_tag(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(EntityType, Uuid, Uuid)>, PathRejection>,
) -> Response {
    let Ok(Path((kind, entity_id, tag_id))) = path else {
        return bad(id);
    };
    unit(
        id,
        service::assign_tag(
            &ctx,
            ChangeEntityTag {
                entity: Entity {
                    entity_type: kind,
                    id: entity_id,
                },
                tag_id,
            },
        )
        .await,
    )
}
async fn remove_tag(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<(EntityType, Uuid, Uuid)>, PathRejection>,
) -> Response {
    let Ok(Path((kind, entity_id, tag_id))) = path else {
        return bad(id);
    };
    unit(
        id,
        service::remove_tag(
            &ctx,
            ChangeEntityTag {
                entity: Entity {
                    entity_type: kind,
                    id: entity_id,
                },
                tag_id,
            },
        )
        .await,
    )
}
async fn create_link(
    State(ctx): State<AppContext>,
    id: RequestId,
    body: Result<Json<CreateLink>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else { return bad(id) };
    match service::create_link(&ctx, b).await {
        Ok(link) => (StatusCode::CREATED, Json(link)).into_response(),
        Err(e) => error(id, e),
    }
}
async fn list_links(
    State(ctx): State<AppContext>,
    id: RequestId,
    q: Result<Query<LinkQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(q)) = q else { return bad(id) };
    match service::list_links(
        &ctx,
        ListLinks {
            source: Entity {
                entity_type: q.source_type,
                id: q.source_id,
            },
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
async fn remove_link(
    State(ctx): State<AppContext>,
    id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
    q: Result<Query<Version>, QueryRejection>,
) -> Response {
    let (Ok(Path(link_id)), Ok(Query(q))) = (path, q) else {
        return bad(id);
    };
    unit(
        id,
        service::remove_link(
            &ctx,
            RemoveLink {
                id: link_id,
                expected_version: q.expected_version,
            },
        )
        .await,
    )
}
