//! T15 acceptance tests: server-side sessions and write-request protection.
//!
//! The tests run against the real local PostgreSQL test database with a
//! per-test schema, and drive the auth middleware through a custom router so the
//! guard can be verified without the product routes T14 adds later. A missing
//! database is a failure, never a skip.
//!
//! Covered here: idle and absolute expiry boundaries with a sliding renewal
//! capped by the absolute limit, logout revoking exactly one session, the
//! Origin + CSRF checks on writes, the plaintext token never being stored or
//! logged, the cookie attributes, and the five-minute verification window T16
//! consumes.

use std::sync::{Arc, OnceLock};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::{Extension, Router};
use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_api::logging::{self, LogBuffer};
use knoted_api::middleware::auth::{
    CSRF_COOKIE, CSRF_HEADER, CurrentSession, SESSION_COOKIE, attach_session_cookies, require_csrf,
    require_origin, require_session,
};
use knoted_core::auth::sessions::{CreatedSession, Session};
use knoted_core::auth::{self, hash_token, sessions};
use knoted_core::config::{AppConfig, AuthConfig, ConfigError, LogFormat, LogLevel};
use knoted_core::context::{AppContext, FixedClock};
use knoted_core::db::testing::TestDb;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 4, 5, 6, 7)
        .single()
        .expect("the fixed instant is valid")
}

/// Creates the single owner account and returns its id.
async fn owner(pool: &PgPool) -> Uuid {
    let mut tx = pool.begin().await.expect("a transaction must start");
    let owner = auth::ensure_owner(&mut tx)
        .await
        .expect("the owner account must be created");
    tx.commit()
        .await
        .expect("the owner transaction must commit");
    owner.user.id
}

async fn new_session(pool: &PgPool, user_id: Uuid, now: DateTime<Utc>) -> CreatedSession {
    create_session(pool, user_id, now, Some("t15 test agent"), false).await
}

/// A pooled connection, mirroring how a route handler reaches the session API.
async fn connection(pool: &PgPool) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    pool.acquire().await.expect("a pooled connection must open")
}

// The helpers below mirror the production signatures exactly and only supply the
// connection, so the tests exercise the same `&mut PgConnection` contract T14 and
// T16 use from a transaction.
async fn create_session(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
    user_agent: Option<&str>,
    verified: bool,
) -> CreatedSession {
    let mut connection = connection(pool).await;
    sessions::create_session(&mut connection, user_id, now, user_agent, verified)
        .await
        .expect("a session must be created")
}

async fn resolve_session(
    pool: &PgPool,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Option<Session>, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::resolve_session(&mut connection, token, now).await
}

async fn logout(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::logout(&mut connection, user_id, session_id, now).await
}

async fn list_sessions(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
    limit: Option<u32>,
) -> Result<Vec<Session>, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::list_sessions(&mut connection, user_id, now, limit).await
}

async fn revoke_session(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::revoke_session(&mut connection, user_id, session_id, now).await
}

async fn mark_verified(
    pool: &PgPool,
    session_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::mark_verified(&mut connection, session_id, now).await
}

async fn revoke_all_sessions_for_user(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<u64, knoted_core::db::DbError> {
    let mut connection = connection(pool).await;
    sessions::revoke_all_sessions_for_user(&mut connection, user_id, now).await
}

/// A fixed-clock context so the middleware resolves sessions deterministically.
fn context(pool: &PgPool, now: DateTime<Utc>) -> AppContext {
    AppContext::new(pool.clone()).with_clock(Arc::new(FixedClock::new(now)))
}

/// Router exposing the guard's contract: a readable and a writable route.
///
/// The CSRF layer is added first and the session layer second, so the session
/// guard runs first and the CSRF guard can read [`CurrentSession`] from the
/// request extension. This is the exact order T14 must use. The transport layers
/// wrap the guard so every response carries the request id, as in production.
fn guarded_app(ctx: AppContext) -> Router {
    knoted_api::with_transport_layers(
        Router::new()
            .route(
                "/guarded",
                get(identity)
                    .post(identity)
                    .head(identity)
                    .options(identity),
            )
            .layer(from_fn_with_state(AuthConfig::default(), require_csrf))
            .layer(from_fn_with_state(ctx, require_session)),
    )
}

/// Router exposing the anonymous write guard (no session required).
fn anonymous_app(config: AuthConfig) -> Router {
    knoted_api::with_transport_layers(
        Router::new()
            .route(
                "/anonymous",
                get(anonymous)
                    .post(anonymous)
                    .head(anonymous)
                    .options(anonymous),
            )
            .layer(from_fn_with_state(config, require_origin)),
    )
}

async fn anonymous() -> &'static str {
    "ok"
}

/// Router with the CSRF guard alone, to prove it fails closed without a session.
fn csrf_only_app(config: AuthConfig) -> Router {
    knoted_api::with_transport_layers(
        Router::new()
            .route("/csrf-only", axum::routing::post(anonymous))
            .layer(from_fn_with_state(config, require_csrf)),
    )
}

async fn identity(Extension(current): Extension<CurrentSession>) -> String {
    current.session.id.to_string()
}

fn session_request(method: Method, path: &str, created: &CreatedSession) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(
            header::COOKIE,
            format!(
                "{SESSION_COOKIE}={}; {CSRF_COOKIE}={}",
                created.token, created.csrf_token
            ),
        )
        .body(Body::empty())
        .expect("the session request is well formed")
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router must answer every request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        // The guard's probe routes answer with a plain session id; error
        // responses answer with JSON. Keep both readable so a non-JSON body is
        // an assertion failure, not a parse panic.
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, headers, json)
}

fn header_request_id(headers: &HeaderMap) -> Uuid {
    headers
        .get("x-request-id")
        .expect("every response must carry X-Request-Id")
        .to_str()
        .expect("X-Request-Id must be ASCII")
        .parse()
        .expect("X-Request-Id must be a UUID")
}

async fn last_verified_at(pool: &PgPool, session_id: Uuid) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT last_verified_at FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .expect("the session row must exist")
}

/// Process-wide log sink, installed at most once so adding logging assertions
/// cannot make the suite order-dependent.
static LOG_BUFFER: OnceLock<LogBuffer> = OnceLock::new();

fn captured_logs() -> &'static LogBuffer {
    LOG_BUFFER.get_or_init(|| {
        let buffer = LogBuffer::new();
        let sink = buffer.clone();
        logging::init_with_writer(move || sink.clone(), LogLevel::Info, LogFormat::Text)
            .expect("the test logging subscriber must install cleanly");
        buffer
    })
}

#[tokio::test]
async fn t15_1_idle_and_absolute_expiry() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();

    // Both limits are computed from the injected clock, never `Utc::now()`.
    let created = new_session(pool, user, now).await;
    assert_eq!(created.session.idle_expires_at, now + Duration::days(7));
    assert_eq!(
        created.session.absolute_expires_at,
        now + Duration::days(30)
    );
    assert_eq!(created.session.last_seen_at, now);
    assert!(created.session.last_verified_at.is_none());

    // An idle session is usable one second before the idle boundary.
    let almost_idle = new_session(pool, user, now).await;
    let resolved = resolve_session(
        pool,
        &almost_idle.token,
        now + Duration::days(7) - Duration::seconds(1),
    )
    .await
    .expect("resolving a live session must not fail")
    .expect("a session one second before the idle boundary must resolve");
    assert_eq!(
        resolved.last_seen_at,
        now + Duration::days(7) - Duration::seconds(1),
        "resolving must slide `last_seen_at` to the request instant"
    );
    assert_eq!(
        resolved.idle_expires_at,
        now + Duration::days(14) - Duration::seconds(1),
        "an idle renewal must extend the idle limit by seven days"
    );

    // At the idle boundary exactly, the session is already expired.
    let idle_boundary = new_session(pool, user, now).await;
    assert!(
        resolve_session(pool, &idle_boundary.token, now + Duration::days(7))
            .await
            .expect("resolving an expired session must not fail")
            .is_none(),
        "a session whose idle limit equals `now` must be rejected"
    );

    // Sliding renewal keeps a daily-used session alive, but never past the
    // absolute limit: at day 29 the next idle limit clamps to day 30 instead of
    // day 36.
    let sliding = new_session(pool, user, now).await;
    for day in [6, 12, 18, 24, 29] {
        let at = now + Duration::days(day);
        let resolved = resolve_session(pool, &sliding.token, at)
            .await
            .expect("resolving a live session must not fail")
            .unwrap_or_else(|| panic!("day {day} is inside the absolute limit"));
        assert_eq!(resolved.last_seen_at, at);
        assert_eq!(resolved.absolute_expires_at, now + Duration::days(30));
    }
    let resolved = resolve_session(pool, &sliding.token, now + Duration::days(29))
        .await
        .expect("resolving a live session must not fail")
        .expect("day 29 is inside the absolute limit");
    assert_eq!(
        resolved.idle_expires_at, resolved.absolute_expires_at,
        "the sliding idle limit must be capped by the absolute limit"
    );

    // The absolute limit applies even when the idle limit is still fresh.
    let absolute = new_session(pool, user, now).await;
    sqlx::query("UPDATE sessions SET idle_expires_at = $1 WHERE id = $2")
        .bind(now + Duration::days(90))
        .bind(absolute.session.id)
        .execute(pool)
        .await
        .expect("the test must be able to age the row");
    assert!(
        resolve_session(pool, &absolute.token, now + Duration::days(30))
            .await
            .expect("resolving an expired session must not fail")
            .is_none(),
        "a session whose absolute limit equals `now` must be rejected even with a fresh idle limit"
    );

    // One second before the absolute limit a still-fresh idle limit resolves,
    // and the renewal is capped again.
    let almost_absolute = new_session(pool, user, now).await;
    sqlx::query("UPDATE sessions SET idle_expires_at = $1 WHERE id = $2")
        .bind(now + Duration::days(90))
        .bind(almost_absolute.session.id)
        .execute(pool)
        .await
        .expect("the test must be able to age the row");
    let resolved = resolve_session(
        pool,
        &almost_absolute.token,
        now + Duration::days(30) - Duration::seconds(1),
    )
    .await
    .expect("resolving a live session must not fail")
    .expect("a session one second before the absolute boundary must resolve");
    assert_eq!(
        resolved.idle_expires_at, resolved.absolute_expires_at,
        "the final renewal must still be capped by the absolute limit"
    );

    db.drop().await;
}

#[tokio::test]
async fn t15_2_logout_revokes_only_that_session() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();
    let app = guarded_app(context(pool, now));

    // No cookie at all is the plain unauthenticated case.
    let (status, headers, body) = call(
        &app,
        Request::get("/guarded")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["code"], "unauthenticated");
    assert_eq!(
        body["request_id"],
        header_request_id(&headers).to_string(),
        "the 401 body and header must carry the same request id"
    );

    let first = new_session(pool, user, now).await;
    let second = new_session(pool, user, now).await;

    // Both sessions are live and the guard identifies each one.
    let (status, _, body) = call(&app, session_request(Method::GET, "/guarded", &first)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, first.session.id.to_string());
    let (status, _, body) = call(&app, session_request(Method::GET, "/guarded", &second)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, second.session.id.to_string());

    assert!(
        logout(pool, user, first.session.id, now)
            .await
            .expect("logging out must succeed"),
        "the first logout must report a live session as revoked"
    );
    assert!(
        !logout(pool, user, first.session.id, now)
            .await
            .expect("logging out twice must not fail"),
        "a repeated logout must be idempotent and report nothing new"
    );

    // A logout cannot reach another account's session: the account id is part
    // of the predicate, exactly like `revoke_session`.
    assert!(
        !logout(pool, Uuid::new_v4(), second.session.id, now)
            .await
            .expect("a foreign logout must not fail"),
        "a different account must not log the owner out"
    );

    // The revoked token stops working immediately, with the shared error body.
    let (status, headers, body) =
        call(&app, session_request(Method::GET, "/guarded", &first)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["code"], "unauthenticated");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "the 401 must explain the failure: {body}"
    );
    assert_eq!(
        body["request_id"],
        header_request_id(&headers).to_string(),
        "the 401 body and header must carry the same request id"
    );

    // The other device's session is untouched.
    let (status, _, body) = call(&app, session_request(Method::GET, "/guarded", &second)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "logging out one session must not revoke the other: {body}"
    );

    // The listing reflects the revocation and only returns live sessions.
    let listed = list_sessions(pool, user, now, None)
        .await
        .expect("sessions must be listable");
    assert!(
        listed.iter().all(|session| session.id != first.session.id),
        "a revoked session must not be listed"
    );
    assert!(
        listed.iter().any(|session| session.id == second.session.id),
        "the other session must stay listed"
    );

    let revoked: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM sessions WHERE id = $1")
            .bind(first.session.id)
            .fetch_one(pool)
            .await
            .expect("the revoked row must exist");
    assert_eq!(revoked, Some(now));
    let untouched: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM sessions WHERE id = $1")
            .bind(second.session.id)
            .fetch_one(pool)
            .await
            .expect("the live row must exist");
    assert!(
        untouched.is_none(),
        "only the logged-out session may be marked revoked"
    );

    // Revoking another user's session is impossible: the row carries the owner,
    // and a mismatching user id must not touch it.
    assert!(
        !revoke_session(pool, Uuid::new_v4(), second.session.id, now)
            .await
            .expect("revoking must not fail"),
        "a different account must not revoke the owner's session"
    );
    let (status, _, body) = call(&app, session_request(Method::GET, "/guarded", &second)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    db.drop().await;
}

#[tokio::test]
async fn t15_3_cross_site_write_rejected() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();
    let app = guarded_app(context(pool, now));
    let config = AuthConfig::default();

    let session = new_session(pool, user, now).await;
    let other = new_session(pool, user, now).await;
    let cookie = format!(
        "{SESSION_COOKIE}={}; {CSRF_COOKIE}={}",
        session.token, session.csrf_token
    );

    // Same origin plus the matching CSRF token is accepted.
    let request = Request::post("/guarded")
        .header(header::COOKIE, cookie.as_str())
        .header(header::ORIGIN, config.origin.as_str())
        .header(CSRF_HEADER, session.csrf_token.as_str())
        .body(Body::empty())
        .expect("request");
    let (status, _, body) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // Read requests need neither Origin nor CSRF: a cross-site GET carries no
    // state change. HEAD and OPTIONS are equally exempt.
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        let request = Request::builder()
            .method(method.clone())
            .uri("/guarded")
            .header(header::COOKIE, cookie.as_str())
            .body(Body::empty())
            .expect("request");
        let (status, _, body) = call(&app, request).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{method} must not require Origin or CSRF: {body}"
        );
    }

    // A missing or foreign Origin is rejected even with a valid CSRF token.
    for origin in [
        None,
        Some("https://evil.example"),
        Some("https://wlab.132140.xyz"),
    ] {
        let mut request = Request::post("/guarded")
            .header(header::COOKIE, cookie.as_str())
            .header(CSRF_HEADER, session.csrf_token.as_str());
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let (status, headers, body) =
            call(&app, request.body(Body::empty()).expect("request")).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "origin {origin:?} must be rejected: {body}"
        );
        assert_eq!(body["code"], "csrf_failed");
        assert_eq!(
            body["request_id"],
            header_request_id(&headers).to_string(),
            "the 403 body and header must carry the same request id"
        );
    }

    // The Origin match is exact: the scheme and the port are part of it.
    for origin in [
        "http://wlab.132140.xyz:15601",
        "https://wlab.132140.xyz:15602",
        "https://wlab.132140.xyz:15601/",
    ] {
        let request = Request::post("/guarded")
            .header(header::COOKIE, cookie.as_str())
            .header(header::ORIGIN, origin)
            .header(CSRF_HEADER, session.csrf_token.as_str())
            .body(Body::empty())
            .expect("request");
        let (status, _, body) = call(&app, request).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "origin {origin:?} is not the configured origin: {body}"
        );
        assert_eq!(body["code"], "csrf_failed");
    }

    // A missing, wrong or foreign-session CSRF token is rejected.
    for token in [None, Some("not-the-token"), Some(other.csrf_token.as_str())] {
        let mut request = Request::post("/guarded")
            .header(header::COOKIE, cookie.as_str())
            .header(header::ORIGIN, config.origin.as_str());
        if let Some(token) = token {
            request = request.header(CSRF_HEADER, token);
        }
        let (status, _, body) = call(&app, request.body(Body::empty()).expect("request")).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "csrf token {token:?} must be rejected: {body}"
        );
        assert_eq!(body["code"], "csrf_failed");
    }

    // Every write method is covered, not just POST.
    for method in [Method::PUT, Method::PATCH, Method::DELETE] {
        let request = Request::builder()
            .method(method.clone())
            .uri("/guarded")
            .header(header::COOKIE, cookie.as_str())
            .header(header::ORIGIN, config.origin.as_str())
            .body(Body::empty())
            .expect("request");
        let (status, _, body) = call(&app, request).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} without a CSRF token must be rejected: {body}"
        );
        assert_eq!(body["code"], "csrf_failed");
    }

    db.drop().await;
}

#[tokio::test]
async fn t15_4_token_not_stored_plaintext() {
    let db = test_db().await;
    let pool = &db.pool;
    let logs = captured_logs();
    let user = owner(pool).await;
    let now = fixed_now();

    let created = new_session(pool, user, now).await;
    let token = created.token.clone();
    let csrf = created.csrf_token.clone();

    // The plaintext appears neither in `Debug` output nor in any text column.
    let debug = format!("{created:?}");
    assert!(
        !debug.contains(&token),
        "Debug leaked the session token: {debug}"
    );
    assert!(
        !debug.contains(&csrf),
        "Debug leaked the CSRF token: {debug}"
    );

    let stored: (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT token_hash, csrf_token_hash FROM sessions WHERE id = $1")
            .bind(created.session.id)
            .fetch_one(pool)
            .await
            .expect("the session row must be readable");
    assert_eq!(stored.0.len(), 32, "the session digest is SHA-256");
    assert_eq!(stored.1.len(), 32, "the CSRF digest is SHA-256");
    assert_eq!(
        stored.0,
        hash_token(&token),
        "the stored session digest must be SHA-256 of the token"
    );
    assert_eq!(
        stored.1,
        hash_token(&csrf),
        "the stored CSRF digest must be SHA-256 of the CSRF token"
    );
    assert_ne!(
        stored.0,
        token.as_bytes(),
        "the session token must never be stored as plaintext"
    );
    assert_ne!(
        stored.1,
        csrf.as_bytes(),
        "the CSRF token must never be stored as plaintext"
    );

    // Neither secret exists as a bytea value anywhere in the sessions table.
    for secret in [&token, &csrf] {
        let matches: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sessions WHERE token_hash = $1 OR csrf_token_hash = $1",
        )
        .bind(secret.as_bytes())
        .fetch_one(pool)
        .await
        .expect("sessions must be readable");
        assert_eq!(
            matches, 0,
            "the plaintext must not be stored as a digest value"
        );
    }

    // The normal lifecycle logs the events without the secrets.
    resolve_session(pool, &token, now)
        .await
        .expect("resolving the session must not fail")
        .expect("the fresh session must resolve");
    logout(pool, user, created.session.id, now)
        .await
        .expect("logging out must succeed");

    let logs = logs.contents();
    assert!(
        logs.contains("created session"),
        "creating a session must be logged: {logs}"
    );
    assert!(
        logs.contains(created.session.id.to_string().as_str()),
        "the log must identify the session without its token: {logs}"
    );
    assert!(
        !logs.contains(&token),
        "the session token reached the logs: {logs}"
    );
    assert!(
        !logs.contains(&csrf),
        "the CSRF token reached the logs: {logs}"
    );
    assert!(
        !logs.contains("token_hash"),
        "digest values must not be logged either: {logs}"
    );

    // The response text itself must not leak secrets: a request that carries
    // the token logs the request line only.
    let app = guarded_app(context(pool, now));
    let fresh = new_session(pool, user, now).await;
    let (status, _, _) = call(&app, session_request(Method::GET, "/guarded", &fresh)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !logs.contains(&fresh.token),
        "a request log leaked the session token: {logs}"
    );
    assert!(
        !logs.contains(&fresh.csrf_token),
        "a request log leaked the CSRF token: {logs}"
    );

    db.drop().await;
}

#[tokio::test]
async fn t15_5_cookie_attributes() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();
    let created = new_session(pool, user, now).await;

    let mut headers = HeaderMap::new();
    attach_session_cookies(&mut headers, &created, now);

    let cookies: Vec<String> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("a Set-Cookie value must be ASCII")
                .to_owned()
        })
        .collect();
    assert_eq!(
        cookies.len(),
        2,
        "the login response sets exactly two cookies"
    );

    let session_cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{SESSION_COOKIE}=")))
        .expect("the session cookie must be set");
    assert!(
        session_cookie.contains(&format!("{SESSION_COOKIE}={}", created.token)),
        "the session cookie must carry the token: {session_cookie}"
    );
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=2592000",
    ] {
        assert!(
            session_cookie.contains(attribute),
            "the session cookie must set {attribute}: {session_cookie}"
        );
    }

    let csrf_cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{CSRF_COOKIE}=")))
        .expect("the CSRF cookie must be set");
    assert!(
        csrf_cookie.contains(&format!("{CSRF_COOKIE}={}", created.csrf_token)),
        "the CSRF cookie must carry the token: {csrf_cookie}"
    );
    assert!(
        !csrf_cookie.contains("HttpOnly"),
        "the CSRF cookie must be readable by the frontend: {csrf_cookie}"
    );
    for attribute in ["Secure", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
        assert!(
            csrf_cookie.contains(attribute),
            "the CSRF cookie must set {attribute}: {csrf_cookie}"
        );
    }

    // The lifetimes are the configured ones, not a same-day guess.
    assert_eq!(
        (created.session.absolute_expires_at - now).num_seconds(),
        2_592_000,
        "the absolute session lifetime must be 30 days"
    );

    db.drop().await;
}

#[test]
fn t15_7_auth_config_defaults_and_overrides() {
    let dir = tempfile::tempdir().expect("a temporary directory must be created");
    let path = dir.path().join("knoted.toml");
    let database = "[database]\nurl = \"postgres://knoted@127.0.0.1:5432/knoted\"\n";

    // Without an `[auth]` section the deployment defaults apply, so an existing
    // configuration file keeps loading unchanged.
    std::fs::write(&path, database).expect("the configuration must be written");
    let config = AppConfig::load(&path).expect("the defaults must load");
    assert_eq!(config.auth(), &AuthConfig::default());
    assert_eq!(config.auth().rp_id, "wlab.132140.xyz");
    assert_eq!(config.auth().origin, "https://wlab.132140.xyz:15601");

    std::fs::write(
        &path,
        format!("{database}\n[auth]\nrp_id = \"notes.example\"\norigin = \"https://notes.example:8443\"\n"),
    )
    .expect("the configuration must be written");
    let config = AppConfig::load(&path).expect("an explicit auth section must load");
    assert_eq!(config.auth().rp_id, "notes.example");
    assert_eq!(config.auth().origin, "https://notes.example:8443");

    // The RP ID never carries a port, and the origin is compared verbatim, so a
    // trailing slash would silently reject every write. Both are rejected at
    // load time instead.
    for (section, field) in [
        ("rp_id = \"wlab.132140.xyz:15601\"", "auth.rp_id"),
        ("rp_id = \"https://wlab.132140.xyz\"", "auth.rp_id"),
        ("rp_id = \"\"", "auth.rp_id"),
        ("origin = \"https://wlab.132140.xyz:15601/\"", "auth.origin"),
        ("origin = \"wlab.132140.xyz\"", "auth.origin"),
        ("origin = \"http://wlab.132140.xyz:15601\"", "auth.origin"),
        (
            "origin = \"https://wlab.132140.xyz:15601/path\"",
            "auth.origin",
        ),
        (
            "origin = \"https://user@wlab.132140.xyz:15601\"",
            "auth.origin",
        ),
        (
            "origin = \"https://wlab.132140.xyz:15601?x=1\"",
            "auth.origin",
        ),
        (
            "origin = \"https://wlab.132140.xyz:15601#frag\"",
            "auth.origin",
        ),
    ] {
        std::fs::write(&path, format!("{database}\n[auth]\n{section}\n"))
            .expect("the configuration must be written");
        let error = AppConfig::load(&path).expect_err("an invalid auth setting must be rejected");
        assert!(
            matches!(error, ConfigError::Invalid { field: actual, .. } if actual == field),
            "expected an invalid `{field}`, got {error:?}"
        );
    }

    // A typo in the section is a parse error, not a silently ignored default.
    std::fs::write(&path, format!("{database}\n[auth]\nbogus = true\n"))
        .expect("the configuration must be written");
    assert!(matches!(
        AppConfig::load(&path),
        Err(ConfigError::Parse { .. })
    ));
}

#[tokio::test]
async fn t15_6_verification_window() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();
    let window = Duration::minutes(5);

    let created = new_session(pool, user, now).await;
    let session_id = created.session.id;
    assert!(
        !sessions::recently_verified(&created.session, now, window),
        "a fresh login that did not verify a Passkey is not verified"
    );

    assert!(
        mark_verified(pool, session_id, now)
            .await
            .expect("marking the session verified must succeed"),
        "a live session must be marked verified"
    );
    assert_eq!(last_verified_at(pool, session_id).await, Some(now));
    let mut verified = created.session;
    verified.last_verified_at = Some(now);
    assert!(
        sessions::recently_verified(&verified, now, window),
        "a verification at the same instant is inside the window"
    );
    assert!(
        sessions::recently_verified(&verified, now + window - Duration::seconds(1), window),
        "one second inside the window must still count"
    );
    assert!(
        !sessions::recently_verified(&verified, now + window, window),
        "the window is exclusive at five minutes"
    );

    // A backwards clock must not widen the window: a verification stamped in the
    // future does not count.
    assert!(
        !sessions::recently_verified(&verified, now - Duration::seconds(1), window),
        "a future verification timestamp must not be trusted"
    );

    // A revoked session can no longer record a verification.
    assert!(
        logout(pool, user, session_id, now)
            .await
            .expect("logging out must succeed"),
        "the fixture session must be live"
    );
    assert!(
        !mark_verified(pool, session_id, now)
            .await
            .expect("marking a revoked session must not fail"),
        "a revoked session must not report a successful verification"
    );

    // A login that verified at creation is already verified for T16's sensitive
    // operations.
    let strong = create_session(pool, user, now, None, true).await;
    assert_eq!(strong.session.last_verified_at, Some(now));
    assert!(sessions::recently_verified(&strong.session, now, window));

    db.drop().await;
}

#[tokio::test]
async fn t15_8_session_writes_join_the_callers_transaction() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();

    // T14 finishes a ceremony by writing the Passkey and the session together.
    // Both statements therefore share one transaction.
    let mut tx = pool.begin().await.expect("a transaction must start");
    sqlx::query(
        "INSERT INTO passkey_credentials (id, user_id, credential_id, passkey, name)          VALUES ($1, $2, $3, '{}'::jsonb, 'T14 key')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(b"t15-credential".to_vec())
    .execute(&mut *tx)
    .await
    .expect("a Passkey row must be insertable");
    let created = sessions::create_session(&mut tx, user, now, Some("T14 ceremony"), true)
        .await
        .expect("a session must be creatable inside the caller's transaction");
    tx.commit()
        .await
        .expect("the ceremony transaction must commit");

    // The committed transaction left both rows behind, and the session works.
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(created.session.id)
        .fetch_one(pool)
        .await
        .expect("sessions must be readable");
    assert_eq!(stored, 1, "the committed session must exist");
    let passkeys: i64 =
        sqlx::query_scalar("SELECT count(*) FROM passkey_credentials WHERE user_id = $1")
            .bind(user)
            .fetch_one(pool)
            .await
            .expect("passkey_credentials must be readable");
    assert_eq!(passkeys, 1, "the committed Passkey must exist");
    assert!(
        resolve_session(pool, &created.token, now)
            .await
            .expect("resolving must not fail")
            .is_some(),
        "the session created in the transaction must be usable"
    );

    // A failed ceremony rolls the session back with everything else: no orphan
    // session may survive.
    let mut tx = pool.begin().await.expect("a transaction must start");
    sqlx::query(
        "INSERT INTO passkey_credentials (id, user_id, credential_id, passkey, name)          VALUES ($1, $2, $3, '{}'::jsonb, 'rolled back key')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(b"t15-rolled-back".to_vec())
    .execute(&mut *tx)
    .await
    .expect("a Passkey row must be insertable");
    let doomed = sessions::create_session(&mut tx, user, now, None, false)
        .await
        .expect("a session must be creatable inside the transaction");
    tx.rollback()
        .await
        .expect("the failed ceremony must roll back");

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE id = $1")
        .bind(doomed.session.id)
        .fetch_one(pool)
        .await
        .expect("sessions must be readable");
    assert_eq!(stored, 0, "a rolled-back transaction must leave no session");
    let passkeys: i64 =
        sqlx::query_scalar("SELECT count(*) FROM passkey_credentials WHERE user_id = $1")
            .bind(user)
            .fetch_one(pool)
            .await
            .expect("passkey_credentials must be readable");
    assert_eq!(
        passkeys, 1,
        "a rolled-back transaction must leave no Passkey"
    );
    assert!(
        resolve_session(pool, &doomed.token, now)
            .await
            .expect("resolving must not fail")
            .is_none(),
        "a rolled-back session token must be unknown"
    );

    // Recovery (T16) revokes every live session of the account at once.
    let first = new_session(pool, user, now).await;
    let second = new_session(pool, user, now).await;
    assert_eq!(
        revoke_all_sessions_for_user(pool, user, now)
            .await
            .expect("revoking every session must succeed"),
        3,
        "the committed session and both fixtures must be revoked"
    );
    for token in [&created.token, &first.token, &second.token] {
        assert!(
            resolve_session(pool, token, now)
                .await
                .expect("resolving must not fail")
                .is_none(),
            "every session of the account must be revoked"
        );
    }
    assert_eq!(
        revoke_all_sessions_for_user(pool, user, now)
            .await
            .expect("revoking every session twice must not fail"),
        0,
        "a repeated revocation must change nothing"
    );

    db.drop().await;
}

#[tokio::test]
async fn t15_9_anonymous_write_origin_guard() {
    let config = AuthConfig::default();
    let app = anonymous_app(config.clone());

    // An anonymous write from the configured origin is allowed: the ceremony
    // endpoints T14/T16 add are protected by Origin alone, without a session.
    let (status, _, body) = call(
        &app,
        Request::post("/anonymous")
            .header(header::ORIGIN, config.origin.as_str())
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // Missing or foreign origins are rejected in the shared error shape.
    for origin in [
        None,
        Some("https://evil.example"),
        Some("https://wlab.132140.xyz"),
    ] {
        let mut request = Request::post("/anonymous");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let (status, headers, body) =
            call(&app, request.body(Body::empty()).expect("request")).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "origin {origin:?} must be rejected: {body}"
        );
        assert_eq!(body["code"], "csrf_failed");
        assert_eq!(
            body["request_id"],
            header_request_id(&headers).to_string(),
            "the 403 body and header must carry the same request id"
        );
    }

    // Reads stay open without an Origin.
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        let request = Request::builder()
            .method(method.clone())
            .uri("/anonymous")
            .body(Body::empty())
            .expect("request");
        let (status, _, body) = call(&app, request).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{method} must stay readable: {body}"
        );
    }

    // The CSRF guard alone fails closed: without a resolved session there is no
    // digest to compare, so even a same-origin write is rejected.
    let csrf_only = csrf_only_app(config.clone());
    let (status, _, body) = call(
        &csrf_only,
        Request::post("/csrf-only")
            .header(header::ORIGIN, config.origin.as_str())
            .header(CSRF_HEADER, "some-token")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["code"], "unauthenticated");
}

#[tokio::test]
async fn t15_10_session_list_filters_and_caps() {
    let db = test_db().await;
    let pool = &db.pool;
    let user = owner(pool).await;
    let now = fixed_now();

    // Created ten days ago, whose seven-day idle limit has already passed.
    let expired =
        create_session(pool, user, now - Duration::days(10), Some("expired"), false).await;
    let middle = create_session(pool, user, now - Duration::hours(2), Some("middle"), false).await;
    let second = create_session(pool, user, now - Duration::hours(1), Some("second"), false).await;
    let newest = create_session(pool, user, now, Some("newest"), false).await;

    let listed = list_sessions(pool, user, now, None)
        .await
        .expect("sessions must be listable");
    let ids: Vec<Uuid> = listed.iter().map(|session| session.id).collect();
    assert_eq!(
        ids,
        vec![newest.session.id, second.session.id, middle.session.id],
        "the list must be newest first, stably ordered, and skip expired sessions"
    );
    assert!(
        !ids.contains(&expired.session.id),
        "an expired session must not be listed"
    );

    // The query is bounded: the caller can ask for fewer rows, a zero limit is
    // clamped to one and the maximum limit keeps the statement bounded.
    let capped = list_sessions(pool, user, now, Some(2))
        .await
        .expect("sessions must be listable");
    assert_eq!(
        capped
            .iter()
            .map(|session| session.id)
            .collect::<Vec<Uuid>>(),
        vec![newest.session.id, second.session.id],
        "the limit must keep the newest rows"
    );
    assert_eq!(
        list_sessions(pool, user, now, Some(0))
            .await
            .expect("a zero limit must not fail")
            .len(),
        1,
        "a zero limit must be clamped to one row"
    );
    assert_eq!(
        list_sessions(pool, user, now, Some(u32::MAX))
            .await
            .expect("the maximum limit must not fail")
            .len(),
        3,
        "the maximum limit must still bound the query to the existing rows"
    );

    db.drop().await;
}
