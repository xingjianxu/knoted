//! Authenticated unified search.

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{IntoResponse, Response},
    routing::get,
};
use knoted_core::{
    context::AppContext,
    search::service::{self, DomainError, SearchQuery},
};
use serde_json::{Map, Value};

use crate::{
    error::ApiErrorBody,
    middleware::{RequestId, auth::require_session},
};

pub fn router(ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/search", get(search))
        .route_layer(from_fn_with_state(ctx, require_session))
}

fn parse_query(pairs: Vec<(String, String)>) -> Option<SearchQuery> {
    let mut fields = Map::new();
    let mut tags = Vec::new();
    for (key, value) in pairs {
        let value = match key.as_str() {
            "tags" => {
                tags.push(Value::String(value));
                continue;
            }
            "limit" => Value::from(value.parse::<usize>().ok()?),
            "archived" => Value::Bool(value.parse().ok()?),
            "q" | "type" | "project" | "updated_from" | "updated_to" | "sort" | "cursor" => {
                Value::String(value)
            }
            _ => return None,
        };
        if fields.insert(key, value).is_some() {
            return None;
        }
    }
    fields.insert("tags".into(), Value::Array(tags));
    serde_json::from_value(Value::Object(fields)).ok()
}

async fn search(
    State(ctx): State<AppContext>,
    request: RequestId,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let Some(input) = query.ok().and_then(|Query(query)| parse_query(query)) else {
        return invalid(request);
    };
    match service::search(&ctx, input).await {
        Ok(page) => Json(page).into_response(),
        Err(DomainError::InvalidQuery) => invalid(request),
        Err(DomainError::Engine(_) | DomainError::Database(_)) => ApiErrorBody::new(
            "dependency_unavailable",
            "search is unavailable",
            request.get(),
        )
        .into_response(StatusCode::SERVICE_UNAVAILABLE),
    }
}

fn invalid(request: RequestId) -> Response {
    ApiErrorBody::new(
        "bad_request",
        "invalid search query or cursor",
        request.get(),
    )
    .into_response(StatusCode::BAD_REQUEST)
}
