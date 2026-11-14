//! Session and write-request protection.
//!
//! Two guards cooperate, and the order matters: [`require_session`] resolves the
//! `knoted_session` cookie and inserts [`CurrentSession`], then [`require_csrf`]
//! rejects a state-changing request unless its `Origin` is exactly the
//! configured one and its `X-CSRF-Token` matches the session's digest. Both
//! are public so a router can mount them without a business route, which is how
//! the integration tests verify the guard.
//!
//! Failures use the shared [`ApiErrorBody`] with the request's own id: a missing,
//! expired or revoked session answers `401 unauthenticated`, a cross-site or
//! token-less write answers `403 csrf_failed`, and a database failure while
//! resolving the session answers `503 dependency_unavailable`.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use knoted_core::auth::hash_token;
use knoted_core::config::AuthConfig;
use knoted_core::context::AppContext;

pub use knoted_core::auth::sessions;
pub use knoted_core::auth::sessions::{
    CSRF_COOKIE, CSRF_HEADER, SESSION_ABSOLUTE_TTL, SESSION_COOKIE, SESSION_IDLE_TTL, Session,
};

use crate::error::ApiErrorBody;
use crate::middleware::RequestId;

/// Stable error code for a missing or unusable session.
pub const UNAUTHENTICATED: &str = "unauthenticated";
/// Stable error code for a failed `Origin`/CSRF check.
pub const CSRF_FAILED: &str = "csrf_failed";
/// Stable error code for a dependency that cannot answer.
pub const DEPENDENCY_UNAVAILABLE: &str = "dependency_unavailable";

/// The session resolved by [`require_session`].
///
/// It is inserted into the request extension, so a handler reads it with
/// `Extension<CurrentSession>` and never touches the cookie itself.
#[derive(Debug, Clone)]
pub struct CurrentSession {
    /// The live session behind this request.
    pub session: Session,
}

/// Resolves the session cookie and rejects the request when it is not live.
///
/// Mount it outermost when combined with [`require_csrf`]:
///
/// ```ignore
/// router
///     .layer(from_fn_with_state(auth_config, require_csrf))
///     .layer(from_fn_with_state(ctx, require_session))
/// ```
pub async fn require_session(
    State(ctx): State<AppContext>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request_id(&request);
    let Some(token) = cookie_value(request.headers(), SESSION_COOKIE) else {
        tracing::debug!(
            target: "knoted_api::auth",
            request_id = %request_id.get(),
            reason = "missing_session_cookie",
            "request rejected"
        );
        return unauthenticated(request_id);
    };

    match sessions::resolve_session(ctx.pool(), &token, ctx.now()).await {
        Ok(Some(session)) => {
            request.extensions_mut().insert(CurrentSession { session });
            next.run(request).await
        }
        Ok(None) => {
            tracing::debug!(
                target: "knoted_api::auth",
                request_id = %request_id.get(),
                reason = "invalid_session",
                "request rejected"
            );
            unauthenticated(request_id)
        }
        Err(error) => {
            tracing::warn!(
                target: "knoted_api::auth",
                request_id = %request_id.get(),
                error = %error,
                "session lookup failed"
            );
            ApiErrorBody::new(
                DEPENDENCY_UNAVAILABLE,
                "the session could not be checked",
                request_id.get(),
            )
            .into_response(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// Rejects a state-changing request that is cross-site or lacks the CSRF token.
///
/// `GET`, `HEAD` and `OPTIONS` are read-only and pass through; `POST`, `PUT`,
/// `PATCH` and `DELETE` require both an exact `Origin` match and an
/// `X-CSRF-Token` whose SHA-256 equals the session's stored digest. Missing
/// values are rejected, so the check fails closed.
pub async fn require_csrf(
    State(config): State<AuthConfig>,
    request: Request,
    next: Next,
) -> Response {
    let request_id = request_id(&request);
    if !is_write(request.method()) {
        return next.run(request).await;
    }

    let Some(current) = request.extensions().get::<CurrentSession>() else {
        // The guard was mounted without `require_session`; without a session
        // there is nothing to compare, so the request cannot be trusted.
        tracing::warn!(
            target: "knoted_api::auth",
            request_id = %request_id.get(),
            reason = "csr_guard_without_session",
            "request rejected"
        );
        return unauthenticated(request_id);
    };

    let origin_ok = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| origin == config.origin);
    let csrf_ok = request
        .headers()
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|presented| {
            constant_time_eq(&hash_token(presented), &current.session.csrf_token_hash)
        });

    if !(origin_ok && csrf_ok) {
        tracing::debug!(
            target: "knoted_api::auth",
            request_id = %request_id.get(),
            session_id = %current.session.id,
            origin_ok,
            csrf_ok,
            "request rejected"
        );
        return ApiErrorBody::new(
            CSRF_FAILED,
            "the request origin or CSRF token is not valid",
            request_id.get(),
        )
        .into_response(StatusCode::FORBIDDEN);
    }

    next.run(request).await
}

/// Appends the session and CSRF cookies to a response.
///
/// The session cookie is `HttpOnly` so scripts cannot read the bearer token; the
/// CSRF cookie is readable on purpose, because the frontend copies it into the
/// [`CSRF_HEADER`] on writes. Both are `Secure`, `SameSite=Lax`, scoped to `/`
/// and expire with the session's absolute limit, so a browser never holds a
/// cookie the server has already stopped accepting.
pub fn attach_session_cookies(
    headers: &mut HeaderMap,
    created: &sessions::CreatedSession,
    now: DateTime<Utc>,
) {
    let max_age = (created.session.absolute_expires_at - now)
        .num_seconds()
        .max(0);

    let session_cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; Max-Age={max_age}; HttpOnly; Secure; SameSite=Lax",
        created.token
    );
    let csrf_cookie = format!(
        "{CSRF_COOKIE}={}; Path=/; Max-Age={max_age}; Secure; SameSite=Lax",
        created.csrf_token
    );

    // Both values are URL-safe base64, so they are always valid header values.
    headers.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie).expect("a generated cookie is a valid header value"),
    );
    headers.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie).expect("a generated cookie is a valid header value"),
    );
}

/// Reads one cookie value out of all `Cookie` headers on the request.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The request id, if the request-id layer already ran.
fn request_id(request: &Request) -> RequestId {
    request
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_default()
}

/// Whether `method` changes state and therefore needs CSRF protection.
fn is_write(method: &Method) -> bool {
    [Method::POST, Method::PUT, Method::PATCH, Method::DELETE].contains(method)
}

/// Digest comparison that does not return early on the first differing byte.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// A 401 in the shared error shape.
fn unauthenticated(request_id: RequestId) -> Response {
    ApiErrorBody::new(
        UNAUTHENTICATED,
        "a live session cookie is required",
        request_id.get(),
    )
    .into_response(StatusCode::UNAUTHORIZED)
}
