//! Authenticated trash reads; restoring is exclusively owned by domain routes.
use crate::{
    error::ApiErrorBody,
    middleware::{RequestId, auth::require_session},
};
use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::get,
};
use knoted_core::{
    config::AuthConfig,
    context::AppContext,
    trash::{self, DomainError, EntityType, ListTrash},
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    entity_type: Option<EntityType>,
}
pub fn router(_config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/trash", get(list))
        .route_layer(from_fn_with_state(ctx, require_session))
}
async fn list(
    State(ctx): State<AppContext>,
    id: RequestId,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(_) => return bad(id),
    };
    match trash::list(
        &ctx,
        ListTrash {
            cursor: q.cursor,
            limit: q.limit,
            entity_type: q.entity_type,
        },
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(DomainError::InvalidQuery) => bad(id),
        Err(DomainError::Database(_)) => {
            ApiErrorBody::new("dependency_unavailable", "trash unavailable", id.get())
                .into_response(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
fn bad(id: RequestId) -> Response {
    ApiErrorBody::new("bad_request", "invalid trash parameters", id.get())
        .into_response(StatusCode::BAD_REQUEST)
}
