//! HTTP adapters for the existing session service and OpenAPI contract.
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use knoted_core::auth::sessions::Session;
use knoted_core::auth::{hash_token, sessions};
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;
use crate::middleware::auth::{
    CurrentSession, constant_time_eq, cookie_value, require_csrf, require_session,
};

/// Session guard wraps the CSRF guard so writes can read the resolved session.
pub fn router(config: AuthConfig, ctx: AppContext) -> Router<AppContext> {
    Router::new()
        .route("/auth/session", get(current_session))
        .route("/auth/logout", post(logout))
        .route("/auth/sessions", get(list_sessions))
        .route("/auth/sessions/{session_id}", delete(revoke_session))
        .route_layer(from_fn_with_state(config, require_csrf))
        .route_layer(from_fn_with_state(ctx, require_session))
}

#[derive(Serialize)]
struct SessionDto {
    id: Uuid,
    user_id: Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    idle_expires_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
    current: bool,
}
impl SessionDto {
    fn from_session(session: &Session, current_id: Uuid) -> Self {
        Self {
            id: session.id,
            user_id: session.user_id,
            created_at: session.created_at,
            last_seen_at: session.last_seen_at,
            idle_expires_at: session.idle_expires_at,
            absolute_expires_at: session.absolute_expires_at,
            current: session.id == current_id,
        }
    }
}
#[derive(Serialize)]
struct AuthenticationResult {
    session: SessionDto,
    csrf_token: String,
}
#[derive(Serialize)]
struct SessionPage {
    items: Vec<SessionDto>,
    next_cursor: Option<String>,
}

async fn current_session(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
    headers: HeaderMap,
) -> Response {
    // Normal reads never rotate CSRF, preserving parallel writes and other tabs.
    let csrf = cookie_value(&headers, sessions::CSRF_COOKIE)
        .filter(|token| constant_time_eq(&hash_token(token), &current.session.csrf_token_hash));
    let (csrf_token, recovered) = if let Some(token) = csrf {
        (token, false)
    } else {
        if !current.write_admitted {
            return crate::middleware::auth::maintenance_response(request_id);
        }
        let mut connection = match ctx.pool().acquire().await {
            Ok(c) => c,
            Err(_) => return unavailable(request_id),
        };
        match sessions::rotate_csrf_token(
            &mut connection,
            current.session.user_id,
            current.session.id,
            ctx.now(),
        )
        .await
        {
            Ok(Some(token)) => (token, true),
            Ok(None) => {
                return error(
                    StatusCode::UNAUTHORIZED,
                    "unauthenticated",
                    "the session is no longer live",
                    request_id,
                );
            }
            Err(_) => return unavailable(request_id),
        }
    };
    let mut response = Json(AuthenticationResult {
        session: SessionDto::from_session(&current.session, current.session.id),
        csrf_token: csrf_token.clone(),
    })
    .into_response();
    if recovered {
        let max_age = (current.session.absolute_expires_at - ctx.now())
            .num_seconds()
            .max(0);
        let value = format!(
            "{}={csrf_token}; Path=/; Max-Age={max_age}; Secure; SameSite=Lax",
            sessions::CSRF_COOKIE
        );
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&value).expect("generated CSRF token is header safe"),
        );
    }
    no_store(response)
}

async fn logout(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
) -> Response {
    let mut connection = match ctx.pool().acquire().await {
        Ok(c) => c,
        Err(_) => return unavailable(request_id),
    };
    match sessions::logout(
        &mut connection,
        current.session.user_id,
        current.session.id,
        ctx.now(),
    )
    .await
    {
        Ok(_) => clear_cookies(StatusCode::NO_CONTENT.into_response()),
        Err(_) => unavailable(request_id),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    created_at: DateTime<Utc>,
    id: Uuid,
}
fn encode_cursor(session: &Session) -> String {
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            v: 1,
            created_at: session.created_at,
            id: session.id,
        })
        .expect("cursor is JSON"),
    )
}
fn decode_cursor(raw: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let cursor: Cursor = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(raw).ok()?).ok()?;
    (cursor.v == 1).then_some((cursor.created_at, cursor.id))
}
async fn list_sessions(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let Query(query) = match query {
        Ok(q) => q,
        Err(_) => return invalid_query(request_id),
    };
    let limit = query.limit.unwrap_or(sessions::SESSION_PAGE_DEFAULT_LIMIT);
    if !(1..=sessions::SESSION_PAGE_MAX_LIMIT).contains(&limit) {
        return invalid_query(request_id);
    }
    let cursor = match query.cursor.as_deref().map(decode_cursor) {
        None => None,
        Some(Some(c)) => Some(c),
        Some(None) => return invalid_query(request_id),
    };
    let mut connection = match ctx.pool().acquire().await {
        Ok(c) => c,
        Err(_) => return unavailable(request_id),
    };
    let items = match sessions::list_sessions(
        &mut connection,
        current.session.user_id,
        ctx.now(),
        cursor,
        Some(limit),
    )
    .await
    {
        Ok(items) => items,
        Err(_) => return unavailable(request_id),
    };
    // The core clamps limit to 100, so probe one older row after a full page
    // instead of requesting limit+1 and silently truncating a 100-row page.
    let next_cursor = if items.len() == limit as usize {
        let last = items.last().expect("limit is nonzero");
        match sessions::list_sessions(
            &mut connection,
            current.session.user_id,
            ctx.now(),
            Some((last.created_at, last.id)),
            Some(1),
        )
        .await
        {
            Ok(next) if !next.is_empty() => Some(encode_cursor(last)),
            Ok(_) => None,
            Err(_) => return unavailable(request_id),
        }
    } else {
        None
    };
    no_store(
        Json(SessionPage {
            items: items
                .iter()
                .map(|s| SessionDto::from_session(s, current.session.id))
                .collect(),
            next_cursor,
        })
        .into_response(),
    )
}
async fn revoke_session(
    State(ctx): State<AppContext>,
    Extension(current): Extension<CurrentSession>,
    request_id: RequestId,
    path: Result<Path<Uuid>, PathRejection>,
) -> Response {
    let Path(id) = match path {
        Ok(path) => path,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "the session id is not valid",
                request_id,
            );
        }
    };
    let mut connection = match ctx.pool().acquire().await {
        Ok(c) => c,
        Err(_) => return unavailable(request_id),
    };
    match sessions::revoke_session(&mut connection, current.session.user_id, id, ctx.now()).await {
        Ok(true) if id == current.session.id => {
            clear_cookies(StatusCode::NO_CONTENT.into_response())
        }
        Ok(true) => no_store(StatusCode::NO_CONTENT.into_response()),
        Ok(false) => error(
            StatusCode::NOT_FOUND,
            "not_found",
            "the session does not exist",
            request_id,
        ),
        Err(_) => unavailable(request_id),
    }
}
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
fn clear_cookies(mut response: Response) -> Response {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "knoted_session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax",
        ),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static("knoted_csrf=; Path=/; Max-Age=0; Secure; SameSite=Lax"),
    );
    no_store(response)
}
fn error(status: StatusCode, code: &str, message: &str, request_id: RequestId) -> Response {
    no_store(ApiErrorBody::new(code, message, request_id.get()).into_response(status))
}
fn invalid_query(request_id: RequestId) -> Response {
    error(
        StatusCode::BAD_REQUEST,
        "validation_failed",
        "the cursor or limit is not valid",
        request_id,
    )
}
fn unavailable(request_id: RequestId) -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_unavailable",
        "the session operation could not complete",
        request_id,
    )
}
