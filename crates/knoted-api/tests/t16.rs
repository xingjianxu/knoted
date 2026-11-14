//! T16 acceptance tests: Passkey credential management and account recovery.
//!
//! The tests run against the real local PostgreSQL test database with a
//! per-test schema, drive the production router with an injectable clock, and
//! use `webauthn-authenticator-rs`'s `SoftToken` for every ceremony. Recovery is
//! exercised end-to-end through HTTP, so the assertions cover the real
//! `webauthn-rs` verification, the single transaction that replaces the
//! credential, and the revocation of every old session.
//!
//! Covered here: recovery touching only the owner, old sessions becoming
//! unusable afterwards, the five-minute re-verification window on sensitive
//! credential operations, the refusal to remove the last Passkey, and a failed
//! recovery leaving the old credentials and sessions untouched.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_api::build_router_with_auth;
use knoted_api::middleware::auth::{CSRF_HEADER, SESSION_COOKIE};
use knoted_core::auth::sessions::{self, CreatedSession};
use knoted_core::auth::{self, DomainAuthError, TokenPurpose};
use knoted_core::config::AuthConfig;
use knoted_core::context::{AppContext, FixedClock};
use knoted_core::db::testing::TestDb;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use webauthn_authenticator_rs::prelude::{
    CreationChallengeResponse, RegisterPublicKeyCredential, Url, WebauthnAuthenticator,
};
use webauthn_authenticator_rs::softtoken::SoftToken;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

const BOOTSTRAP_OPTIONS: &str = "/api/v1/auth/bootstrap/options";
const BOOTSTRAP_FINISH: &str = "/api/v1/auth/bootstrap/finish";
const PASSKEYS: &str = "/api/v1/auth/passkeys";
const PASSKEYS_OPTIONS: &str = "/api/v1/auth/passkeys/options";
const PASSKEYS_FINISH: &str = "/api/v1/auth/passkeys/finish";
const RECOVERY_OPTIONS: &str = "/api/v1/auth/recovery/options";
const RECOVERY_FINISH: &str = "/api/v1/auth/recovery/finish";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 6, 7, 8, 9)
        .single()
        .expect("the fixed instant is valid")
}

fn context(pool: &PgPool, clock: &Arc<FixedClock>) -> AppContext {
    AppContext::new(pool.clone()).with_clock(clock.clone())
}

fn config() -> AuthConfig {
    AuthConfig::default()
}

fn origin(config: &AuthConfig) -> Url {
    Url::parse(&config.origin).expect("the configured origin is a valid URL")
}

/// A SoftToken with `falsify_uv = true`: the passkey flows require user
/// verification and this fake authenticator reports it.
fn soft_token() -> SoftToken {
    SoftToken::new(true)
        .expect("the software authenticator must initialise")
        .0
}

/// The browser's view of a session: the cookie to send and the CSRF token to
/// echo on writes.
#[derive(Clone)]
struct Browser {
    cookie: String,
    csrf: String,
}

impl Browser {
    /// Reads the two `Set-Cookie` headers and the CSRF token from an
    /// `AuthenticationResult` response.
    fn from_response(headers: &HeaderMap, body: &Value) -> Browser {
        let mut session_cookie = None;
        let mut csrf_cookie = None;
        for pair in headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|cookie| cookie.split(';').next())
            .filter_map(|pair| pair.split_once('='))
        {
            match pair.0 {
                SESSION_COOKIE => session_cookie = Some(pair.1.to_owned()),
                "knoted_csrf" => csrf_cookie = Some(pair.1.to_owned()),
                _ => {}
            }
        }
        let session_cookie = session_cookie.expect("the response must set the session cookie");
        let csrf_cookie = csrf_cookie.expect("the response must set the CSRF cookie");
        assert_eq!(
            body["csrf_token"], csrf_cookie,
            "the body and cookie must carry the same CSRF token"
        );
        Browser {
            cookie: format!("{SESSION_COOKIE}={session_cookie}; knoted_csrf={csrf_cookie}"),
            csrf: csrf_cookie,
        }
    }
}

fn json_request(path: &str, body: &Value, origin: &str) -> Request<Body> {
    Request::post(path)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "t16 test agent")
        .body(Body::from(
            serde_json::to_vec(body).expect("the test body is JSON"),
        ))
        .expect("the request is well formed")
}

/// An authenticated write: session cookie, CSRF header and an exact origin.
fn authenticated_json(path: &str, session: &Browser, body: &Value, origin: &str) -> Request<Body> {
    Request::post(path)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &session.cookie)
        .header(CSRF_HEADER, &session.csrf)
        .header(header::USER_AGENT, "t16 test agent")
        .body(Body::from(
            serde_json::to_vec(body).expect("the test body is JSON"),
        ))
        .expect("the request is well formed")
}

/// An authenticated write without a body (the options endpoints).
fn authenticated_empty(
    path: &str,
    method: Method,
    session: &Browser,
    origin: &str,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &session.cookie)
        .header(CSRF_HEADER, &session.csrf)
        .header(header::USER_AGENT, "t16 test agent")
        .body(Body::empty())
        .expect("the request is well formed")
}

/// An authenticated read, without a CSRF header (the guard only covers writes).
fn authenticated_get(path: &str, session: &Browser, origin: &str) -> Request<Body> {
    Request::get(path)
        .header(header::ORIGIN, origin)
        .header(header::COOKIE, &session.cookie)
        .header(header::USER_AGENT, "t16 test agent")
        .body(Body::empty())
        .expect("the request is well formed")
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
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, json)
}

async fn issue_token(pool: &PgPool, purpose: TokenPurpose, now: DateTime<Utc>) -> String {
    auth::issue_token(pool, purpose, now)
        .await
        .expect("a credential token must be issued")
        .plaintext()
        .to_owned()
}

/// Creates a session directly, the way a login would, so a test can control
/// `last_verified_at` and keep the plaintext token.
async fn new_session(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
    verified: bool,
) -> CreatedSession {
    let mut connection = pool.acquire().await.expect("a pooled connection must open");
    sessions::create_session(
        &mut connection,
        user_id,
        now,
        Some("t16 old agent"),
        verified,
    )
    .await
    .expect("a session must be created")
}

fn browser_from_created(created: &CreatedSession) -> Browser {
    Browser {
        cookie: format!(
            "{SESSION_COOKIE}={}; knoted_csrf={}",
            created.token, created.csrf_token
        ),
        csrf: created.csrf_token.clone(),
    }
}

/// Registers the owner through the real bootstrap route and returns the
/// response body plus the new browser session.
async fn bootstrap_owner(
    app: &Router,
    config: &AuthConfig,
    pool: &PgPool,
    now: DateTime<Utc>,
) -> (Value, Browser) {
    let token = issue_token(pool, TokenPurpose::Bootstrap, now).await;
    let (status, _, body) = call(
        app,
        json_request(
            BOOTSTRAP_OPTIONS,
            &json!({ "bootstrap_token": token }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bootstrap options failed: {body}");
    let ceremony_id: Uuid = body["ceremony_id"]
        .as_str()
        .expect("the response must carry a ceremony id")
        .parse()
        .expect("the ceremony id must be a UUID");
    let challenge: CreationChallengeResponse = serde_json::from_value(body["options"].clone())
        .expect("the options must deserialize as a creation challenge");
    let mut authenticator = soft_token();
    let credential = authenticator
        .do_registration(origin(config), challenge)
        .expect("the software authenticator must complete registration");
    let credential = serde_json::to_value(&credential).expect("the credential is JSON");

    let (status, headers, body) = call(
        app,
        json_request(
            BOOTSTRAP_FINISH,
            &json!({ "ceremony_id": ceremony_id, "credential": credential }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bootstrap finish failed: {body}"
    );
    let browser = Browser::from_response(&headers, &body);
    (body, browser)
}

/// Adds a second Passkey through the authenticated route and returns the new
/// record's body.
async fn add_passkey(
    app: &Router,
    config: &AuthConfig,
    session: &Browser,
    name: &str,
) -> (StatusCode, Value) {
    let (status, _, body) = call(
        app,
        authenticated_empty(PASSKEYS_OPTIONS, Method::POST, session, &config.origin),
    )
    .await;
    if status != StatusCode::OK {
        return (status, body);
    }
    let ceremony_id: Uuid = body["ceremony_id"]
        .as_str()
        .expect("the response must carry a ceremony id")
        .parse()
        .expect("the ceremony id must be a UUID");
    let challenge: CreationChallengeResponse = serde_json::from_value(body["options"].clone())
        .expect("the options must deserialize as a creation challenge");
    let mut authenticator = soft_token();
    let credential = authenticator
        .do_registration(origin(config), challenge)
        .expect("the software authenticator must complete registration");
    let credential = serde_json::to_value(&credential).expect("the credential is JSON");

    let (status, _, body) = call(
        app,
        authenticated_json(
            PASSKEYS_FINISH,
            session,
            &json!({
                "ceremony_id": ceremony_id,
                "credential": credential,
                "name": name,
            }),
            &config.origin,
        ),
    )
    .await;
    (status, body)
}

/// Runs `recovery/options` and `recovery/finish` with a fresh SoftToken.
async fn recover(
    app: &Router,
    config: &AuthConfig,
    token: &str,
    name: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let (status, _, body) = call(
        app,
        json_request(
            RECOVERY_OPTIONS,
            &json!({ "recovery_token": token }),
            &config.origin,
        ),
    )
    .await;
    if status != StatusCode::OK {
        return (status, HeaderMap::new(), body);
    }
    let ceremony_id: Uuid = body["ceremony_id"]
        .as_str()
        .expect("the response must carry a ceremony id")
        .parse()
        .expect("the ceremony id must be a UUID");
    let challenge: CreationChallengeResponse = serde_json::from_value(body["options"].clone())
        .expect("the options must deserialize as a creation challenge");
    let mut authenticator = soft_token();
    let credential: RegisterPublicKeyCredential = authenticator
        .do_registration(origin(config), challenge)
        .expect("the software authenticator must complete registration");
    let credential = serde_json::to_value(&credential).expect("the credential is JSON");

    let mut finish = json!({
        "recovery_token": token,
        "ceremony_id": ceremony_id,
        "credential": credential,
    });
    if let Some(name) = name {
        finish["name"] = json!(name);
    }
    call(app, json_request(RECOVERY_FINISH, &finish, &config.origin)).await
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    let (count,): (i64,) = sqlx::query_as(sql)
        .fetch_one(pool)
        .await
        .expect("the count query must succeed");
    count
}

async fn passkey_ids(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM passkey_credentials ORDER BY created_at, id")
        .fetch_all(pool)
        .await
        .expect("the passkey rows must be readable")
}

async fn credential_id_bytes(pool: &PgPool, id: Uuid) -> Vec<u8> {
    sqlx::query_scalar("SELECT credential_id FROM passkey_credentials WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the credential id must be readable")
}

async fn load_passkey_page(app: &Router, session: &Browser, origin: &str) -> (StatusCode, Value) {
    let (status, _, body) = call(app, authenticated_get(PASSKEYS, session, origin)).await;
    (status, body)
}

#[tokio::test]
async fn t16_1_recovery_affects_only_owner() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (bootstrapped, _bootstrap_browser) = bootstrap_owner(&app, &config, pool, now).await;
    let owner_id: Uuid = bootstrapped["session"]["user_id"]
        .as_str()
        .expect("the owner id is a string")
        .parse()
        .expect("the owner id is a UUID");
    let old_credentials = passkey_ids(pool).await;
    assert_eq!(old_credentials.len(), 1, "one Passkey before recovery");
    let old_credential_id = credential_id_bytes(pool, old_credentials[0]).await;

    // An extra session that predates the recovery, like another device.
    let old_session = new_session(pool, owner_id, now, true).await;
    let token = issue_token(pool, TokenPurpose::Recovery, now).await;

    let (status, headers, body) = recover(&app, &config, &token, Some("New Passkey")).await;
    assert_eq!(status, StatusCode::CREATED, "recovery failed: {body}");
    assert_eq!(body["session"]["user_id"], owner_id.to_string());
    assert_eq!(body["session"]["current"], true);
    let recovery_browser = Browser::from_response(&headers, &body);

    // Exactly one owner and exactly one credential: the replacement.
    assert_eq!(
        count(pool, "SELECT count(*) FROM users").await,
        1,
        "recovery must not create a second account"
    );
    let new_credentials = passkey_ids(pool).await;
    assert_eq!(new_credentials.len(), 1, "the old credentials are removed");
    assert_ne!(
        new_credentials[0], old_credentials[0],
        "the replacement is a new credential row"
    );
    assert_ne!(
        credential_id_bytes(pool, new_credentials[0]).await,
        old_credential_id,
        "the replacement has a different credential id"
    );
    let (name,): (String,) = sqlx::query_as("SELECT name FROM passkey_credentials WHERE id = $1")
        .bind(new_credentials[0])
        .fetch_one(pool)
        .await
        .expect("the new passkey must exist");
    assert_eq!(name, "New Passkey");

    // Every old session is revoked and the recovery session is the only live one.
    let live = count(
        pool,
        "SELECT count(*) FROM sessions WHERE revoked_at IS NULL",
    )
    .await;
    assert_eq!(live, 1, "only the recovery session stays live");
    let (old_revoked,): (Option<DateTime<Utc>>,) =
        sqlx::query_as("SELECT revoked_at FROM sessions WHERE id = $1")
            .bind(old_session.session.id)
            .fetch_one(pool)
            .await
            .expect("the old session row must remain");
    assert_eq!(old_revoked, Some(now), "the old session is revoked at now");
    let page_status = load_passkey_page(&app, &recovery_browser, &config.origin).await;
    assert_eq!(page_status.0, StatusCode::OK);

    // Only the issued recovery token is stored for that purpose, and it is
    // consumed; the earlier bootstrap token row is untouched by recovery.
    let (recovery_rows, consumed): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE consumed_at IS NOT NULL) \
         FROM credential_tokens WHERE purpose = 'recovery'",
    )
    .fetch_one(pool)
    .await
    .expect("the token table must be readable");
    assert_eq!(recovery_rows, 1, "only the issued recovery token is stored");
    assert_eq!(consumed, 1, "the recovery token is consumed");

    db.drop().await;
}

#[tokio::test]
async fn t16_2_old_sessions_invalid_after_recovery() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (bootstrapped, bootstrap_browser) = bootstrap_owner(&app, &config, pool, now).await;
    let owner_id: Uuid = bootstrapped["session"]["user_id"]
        .as_str()
        .expect("the owner id is a string")
        .parse()
        .expect("the owner id is a UUID");
    // The bootstrap session works before the recovery, and so does an extra
    // session created for the owner.
    assert_eq!(
        load_passkey_page(&app, &bootstrap_browser, &config.origin)
            .await
            .0,
        StatusCode::OK
    );
    let old_session = new_session(pool, owner_id, now, true).await;
    let old_browser = browser_from_created(&old_session);
    assert_eq!(
        load_passkey_page(&app, &old_browser, &config.origin)
            .await
            .0,
        StatusCode::OK
    );

    let token = issue_token(pool, TokenPurpose::Recovery, now).await;
    let (status, headers, body) = recover(&app, &config, &token, None).await;
    assert_eq!(status, StatusCode::CREATED, "recovery failed: {body}");
    let recovery_browser = Browser::from_response(&headers, &body);

    // The old sessions are rejected even though their rows still exist.
    let (status, body) = load_passkey_page(&app, &bootstrap_browser, &config.origin).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert_eq!(body["code"], "unauthenticated");
    let (status, _) = load_passkey_page(&app, &old_browser, &config.origin).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The session returned by the recovery is live and belongs to the owner.
    let (status, page) = load_passkey_page(&app, &recovery_browser, &config.origin).await;
    assert_eq!(status, StatusCode::OK, "page: {page}");
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(page["items"][0]["name"], "Passkey");

    db.drop().await;
}

#[tokio::test]
async fn t16_3_recent_verification_required() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (_bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;

    // Right after the login ceremony the credential operations are allowed.
    let (status, body) = add_passkey(&app, &config, &browser, "Second").await;
    assert_eq!(status, StatusCode::CREATED, "add passkey failed: {body}");
    assert_eq!(passkey_ids(pool).await.len(), 2);

    // Six minutes later the verification is stale for both add and remove.
    clock.advance(Duration::minutes(6));
    let (status, _, body) = call(
        &app,
        authenticated_empty(PASSKEYS_OPTIONS, Method::POST, &browser, &config.origin),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["code"], "recent_verification_required");

    let doomed = passkey_ids(pool).await[0];
    let (status, _, body) = call(
        &app,
        authenticated_empty(
            &format!("{PASSKEYS}/{doomed}"),
            Method::DELETE,
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(body["code"], "recent_verification_required");
    assert_eq!(
        passkey_ids(pool).await.len(),
        2,
        "a stale verification must not delete a credential"
    );

    // Inside the five-minute window the same delete succeeds.
    clock.set(now);
    let (status, _, body) = call(
        &app,
        authenticated_empty(
            &format!("{PASSKEYS}/{doomed}"),
            Method::DELETE,
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body: {body}");
    assert_eq!(passkey_ids(pool).await.len(), 1);

    db.drop().await;
}

#[tokio::test]
async fn t16_4_last_credential_cannot_be_removed() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (_bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;
    let only = passkey_ids(pool).await[0];

    // The single remaining credential cannot be deleted.
    let (status, _, body) = call(
        &app,
        authenticated_empty(
            &format!("{PASSKEYS}/{only}"),
            Method::DELETE,
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "last_credential");
    assert_eq!(passkey_ids(pool).await, vec![only]);

    // With a second credential the first one can go.
    let (status, body) = add_passkey(&app, &config, &browser, "Second").await;
    assert_eq!(status, StatusCode::CREATED, "add passkey failed: {body}");
    assert_eq!(passkey_ids(pool).await.len(), 2);

    let (status, _, body) = call(
        &app,
        authenticated_empty(
            &format!("{PASSKEYS}/{only}"),
            Method::DELETE,
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body: {body}");
    let remaining = passkey_ids(pool).await;
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0], only);

    // A foreign id is a 404, and a second delete of the removed id is a 404 too.
    let (status, _, body) = call(
        &app,
        authenticated_empty(
            &format!("{PASSKEYS}/{}", UUID_NOT_FOUND),
            Method::DELETE,
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(
        body["code"],
        DomainAuthError::PasskeyNotFound.reason(),
        "the HTTP code and the domain reason must agree"
    );

    db.drop().await;
}

const UUID_NOT_FOUND: Uuid = Uuid::nil();

#[tokio::test]
async fn t16_5_failed_recovery_keeps_old_credentials() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (_bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;
    let original = passkey_ids(pool).await;
    assert_eq!(original.len(), 1);

    // 1. An unknown token never even creates a ceremony.
    let (status, _, body) = call(
        &app,
        json_request(
            RECOVERY_OPTIONS,
            &json!({ "recovery_token": "not-a-real-recovery-token" }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "invalid_token");

    // 2. An expired token is refused with its own stable code.
    let expired = issue_token(pool, TokenPurpose::Recovery, now).await;
    clock.advance(Duration::minutes(11));
    let (status, _, body) = call(
        &app,
        json_request(
            RECOVERY_OPTIONS,
            &json!({ "recovery_token": expired }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "expired_token");
    clock.set(now);

    // 3. A genuine ceremony whose credential is tampered with is rejected
    //    before the transaction, so the token stays usable.
    let token = issue_token(pool, TokenPurpose::Recovery, now).await;
    let (ceremony_id, options) = {
        let (status, _, body) = call(
            &app,
            json_request(
                RECOVERY_OPTIONS,
                &json!({ "recovery_token": token }),
                &config.origin,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "recovery options failed: {body}");
        (
            body["ceremony_id"]
                .as_str()
                .expect("the ceremony id is a string")
                .parse::<Uuid>()
                .expect("the ceremony id is a UUID"),
            body["options"].clone(),
        )
    };
    let challenge: CreationChallengeResponse =
        serde_json::from_value(options).expect("the options must be a creation challenge");
    let mut authenticator = soft_token();
    let credential = authenticator
        .do_registration(origin(&config), challenge)
        .expect("the software authenticator must complete registration");
    // Tamper with the signed client data: the origin changes without a matching
    // signature, so verification must reject it. The typed struct is used so
    // the WebAuthn JSON field names stay correct.
    let mut tampered = credential.clone();
    let mut client_data: Value = serde_json::from_slice(&tampered.response.client_data_json)
        .expect("the client data must be JSON");
    client_data["origin"] = json!("https://evil.example");
    tampered.response.client_data_json =
        serde_json::to_vec(&client_data).expect("the tampered client data is JSON");
    let tampered = serde_json::to_value(&tampered).expect("the credential is JSON");

    let (status, _, body) = call(
        &app,
        json_request(
            RECOVERY_FINISH,
            &json!({
                "recovery_token": token,
                "ceremony_id": ceremony_id,
                "credential": tampered,
            }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "credential_rejected");

    // Nothing changed: the old credential, the old session and the token.
    assert_eq!(passkey_ids(pool).await, original);
    assert_eq!(
        load_passkey_page(&app, &browser, &config.origin).await.0,
        StatusCode::OK,
        "the old session must still work"
    );
    let (consumed,): (Option<DateTime<Utc>>,) =
        sqlx::query_as("SELECT consumed_at FROM credential_tokens WHERE token_hash = $1")
            .bind(auth::hash_token(&token).as_slice())
            .fetch_one(pool)
            .await
            .expect("the recovery token row must exist");
    assert_eq!(
        consumed, None,
        "a rejected credential must not burn the token"
    );
    assert_eq!(
        count(pool, "SELECT count(*) FROM users").await,
        1,
        "the owner is untouched"
    );

    db.drop().await;
}

/// Builds a request with independent control over the session cookie, the
/// `Origin` header and the CSRF header, so the guards can be tested one at a
/// time.
fn guarded_request(
    method: Method,
    path: &str,
    session: Option<&Browser>,
    origin: Option<&str>,
    csrf: bool,
    body: Option<&Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(session) = session {
        builder = builder.header(header::COOKIE, &session.cookie);
        if csrf {
            builder = builder.header(CSRF_HEADER, &session.csrf);
        }
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(body).expect("the test body is JSON"))
        }
        None => Body::empty(),
    };
    builder
        .header(header::USER_AGENT, "t16 test agent")
        .body(body)
        .expect("the request is well formed")
}

/// Parses a `PasskeyPage` body into its ids and next cursor.
fn page_ids(body: &Value) -> (Vec<Uuid>, Option<String>) {
    let items = body["items"].as_array().expect("the page must carry items");
    let ids = items
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .expect("a Passkey id is a string")
                .parse()
                .expect("a Passkey id is a UUID")
        })
        .collect();
    let next_cursor = body["next_cursor"].as_str().map(str::to_owned);
    (ids, next_cursor)
}

#[tokio::test]
async fn t16_6_passkey_page_paginates() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (_bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;
    for name in ["Second", "Third"] {
        let (status, body) = add_passkey(&app, &config, &browser, name).await;
        assert_eq!(status, StatusCode::CREATED, "add passkey failed: {body}");
    }
    let mut all = passkey_ids(pool).await;
    assert_eq!(all.len(), 3, "three credentials must exist");
    all.sort_unstable();

    // A full page carries a cursor, the following page carries the remainder
    // and no cursor.
    let (status, _, first) = call(
        &app,
        authenticated_get(&format!("{PASSKEYS}?limit=2"), &browser, &config.origin),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first page failed: {first}");
    let (first_ids, cursor) = page_ids(&first);
    assert_eq!(first_ids.len(), 2, "the limit must bound the page");
    let cursor = cursor.expect("a full page must carry a next cursor");

    let (status, _, second) = call(
        &app,
        authenticated_get(
            &format!("{PASSKEYS}?limit=2&cursor={cursor}"),
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "second page failed: {second}");
    let (second_ids, next_cursor) = page_ids(&second);
    assert_eq!(second_ids.len(), 1, "the second page holds the rest");
    assert_eq!(next_cursor, None, "the last page has no cursor");

    let mut walked: Vec<Uuid> = first_ids.iter().chain(second_ids.iter()).copied().collect();
    assert_eq!(walked.len(), 3, "paging must not repeat a credential");
    walked.sort_unstable();
    assert_eq!(
        walked, all,
        "paging must neither repeat nor skip a credential"
    );

    // A page larger than the list is a single page with no cursor.
    let (status, _, page) = call(
        &app,
        authenticated_get(&format!("{PASSKEYS}?limit=100"), &browser, &config.origin),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "single page failed: {page}");
    let (ids, cursor) = page_ids(&page);
    assert_eq!(ids.len(), 3, "the default page fits every credential");
    assert_eq!(cursor, None, "there is no next page");

    // A limit outside the contract's range is a client error with a stable
    // code, not a silently clamped page.
    for query in ["limit=0", "limit=101"] {
        let (status, _, body) = call(
            &app,
            authenticated_get(&format!("{PASSKEYS}?{query}"), &browser, &config.origin),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        assert_eq!(body["code"], "validation_failed", "{query}: {body}");
        assert_eq!(body["field_errors"][0]["field"], "limit", "{query}: {body}");
    }
    // A non-numeric limit is rejected by the extractor, and a malformed cursor
    // is rejected explicitly instead of silently restarting the listing.
    let (status, _, body) = call(
        &app,
        authenticated_get(&format!("{PASSKEYS}?limit=abc"), &browser, &config.origin),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "limit=abc: {body}");
    assert_eq!(body["code"], "invalid_request", "limit=abc: {body}");

    let (status, _, body) = call(
        &app,
        authenticated_get(
            &format!("{PASSKEYS}?cursor=not-a-cursor"),
            &browser,
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad cursor: {body}");
    assert_eq!(body["code"], "validation_failed", "bad cursor: {body}");
    assert_eq!(
        body["field_errors"][0]["field"], "cursor",
        "bad cursor: {body}"
    );

    db.drop().await;
}

#[tokio::test]
async fn t16_7_recovery_rollback_on_duplicate_credential() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;
    let owner_id: Uuid = bootstrapped["session"]["user_id"]
        .as_str()
        .expect("the owner id is a string")
        .parse()
        .expect("the owner id is a UUID");
    let original = passkey_ids(pool).await;
    assert_eq!(original.len(), 1);

    let token = issue_token(pool, TokenPurpose::Recovery, now).await;
    let (status, _, body) = call(
        &app,
        json_request(
            RECOVERY_OPTIONS,
            &json!({ "recovery_token": token }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "recovery options failed: {body}");
    let ceremony_id: Uuid = body["ceremony_id"]
        .as_str()
        .expect("the ceremony id is a string")
        .parse()
        .expect("the ceremony id is a UUID");
    let challenge: CreationChallengeResponse = serde_json::from_value(body["options"].clone())
        .expect("the options must be a creation challenge");
    let mut authenticator = soft_token();
    let credential = authenticator
        .do_registration(origin(&config), challenge)
        .expect("the software authenticator must complete registration");
    // The recovery is about to install this credential id. Pre-occupying it
    // makes the insert inside the transaction fail on the unique constraint.
    let squatted_id = credential.raw_id.clone();
    sqlx::query(
        "INSERT INTO passkey_credentials (id, user_id, credential_id, passkey, name, created_at) \
         VALUES ($1, $2, $3, '{}'::jsonb, 'Squatter', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(owner_id)
    .bind(&squatted_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("the squatting row must be inserted");

    let credential_json = serde_json::to_value(&credential).expect("the credential is JSON");
    let finish = json!({
        "recovery_token": token,
        "ceremony_id": ceremony_id,
        "credential": credential_json,
    });
    let (status, _, body) =
        call(&app, json_request(RECOVERY_FINISH, &finish, &config.origin)).await;
    assert_eq!(status, StatusCode::CONFLICT, "finish: {body}");
    assert_eq!(body["code"], "duplicate_credential");

    // The whole transaction rolled back: neither the ceremony nor the token is
    // consumed, and the old credential and session keep working.
    let (token_consumed,): (Option<DateTime<Utc>>,) =
        sqlx::query_as("SELECT consumed_at FROM credential_tokens WHERE token_hash = $1")
            .bind(auth::hash_token(&token).as_slice())
            .fetch_one(pool)
            .await
            .expect("the recovery token row must exist");
    assert_eq!(
        token_consumed, None,
        "a rolled-back recovery must not consume the token"
    );
    let (ceremony_consumed,): (Option<DateTime<Utc>>,) =
        sqlx::query_as("SELECT consumed_at FROM ceremonies WHERE id = $1")
            .bind(ceremony_id)
            .fetch_one(pool)
            .await
            .expect("the ceremony row must exist");
    assert_eq!(
        ceremony_consumed, None,
        "a rolled-back recovery must not consume the ceremony"
    );
    let after_failure = passkey_ids(pool).await;
    assert!(
        after_failure.contains(&original[0]),
        "the old credential must survive a failed recovery"
    );
    let (status, page) = load_passkey_page(&app, &browser, &config.origin).await;
    assert_eq!(status, StatusCode::OK, "old session page: {page}");

    // Remove the conflicting row and retry the same request: the ceremony and
    // token are still usable, so it succeeds and the old session is revoked.
    sqlx::query("DELETE FROM passkey_credentials WHERE credential_id = $1")
        .bind(&squatted_id)
        .execute(pool)
        .await
        .expect("the squatting row must be deletable");
    let (status, headers, body) =
        call(&app, json_request(RECOVERY_FINISH, &finish, &config.origin)).await;
    assert_eq!(status, StatusCode::CREATED, "retried finish: {body}");
    let recovery_browser = Browser::from_response(&headers, &body);
    let (status, body) = load_passkey_page(&app, &browser, &config.origin).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the old session must be revoked after a successful recovery: {body}"
    );
    assert_eq!(body["code"], "unauthenticated");
    assert_eq!(
        load_passkey_page(&app, &recovery_browser, &config.origin)
            .await
            .0,
        StatusCode::OK,
        "the recovery session must be usable"
    );
    let replaced = passkey_ids(pool).await;
    assert_eq!(replaced.len(), 1, "recovery replaces the credential set");
    assert_ne!(replaced[0], original[0]);

    db.drop().await;
}

#[tokio::test]
async fn t16_8_origin_and_csrf_guards() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let app = build_router_with_auth(context(pool, &clock), config.clone());

    let (_bootstrapped, browser) = bootstrap_owner(&app, &config, pool, now).await;

    // The anonymous recovery routes reject a missing or foreign Origin.
    let token = issue_token(pool, TokenPurpose::Recovery, now).await;
    let recovery_options = json!({ "recovery_token": token });
    let recovery_finish = json!({
        "recovery_token": token,
        "ceremony_id": Uuid::nil(),
        "credential": {},
    });
    for (path, body) in [
        (RECOVERY_OPTIONS, &recovery_options),
        (RECOVERY_FINISH, &recovery_finish),
    ] {
        for origin_header in [None, Some("https://evil.example")] {
            let (status, _, response) = call(
                &app,
                guarded_request(Method::POST, path, None, origin_header, false, Some(body)),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{path} with origin {origin_header:?}: {response}"
            );
            assert_eq!(response["code"], "csrf_failed");
        }
    }
    // The same recovery options request with the configured Origin succeeds.
    let (status, _, body) = call(
        &app,
        json_request(RECOVERY_OPTIONS, &recovery_options, &config.origin),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "recovery options: {body}");

    // A write without a CSRF token is rejected even with a live session.
    for (method, path, body) in [
        (Method::POST, PASSKEYS_OPTIONS, None),
        (Method::POST, PASSKEYS_FINISH, Some(&json!({}))),
        (
            Method::DELETE,
            &format!("{PASSKEYS}/{}", UUID_NOT_FOUND) as &str,
            None,
        ),
    ] {
        let (status, _, response) = call(
            &app,
            guarded_request(
                method,
                path,
                Some(&browser),
                Some(&config.origin),
                false,
                body,
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} without CSRF: {response}"
        );
        assert_eq!(response["code"], "csrf_failed");
    }
    // A missing session is a 401, even when the origin and a CSRF token are
    // presented.
    for (method, path) in [
        (Method::POST, PASSKEYS_OPTIONS.to_owned()),
        (Method::POST, PASSKEYS_FINISH.to_owned()),
        (Method::DELETE, format!("{PASSKEYS}/{UUID_NOT_FOUND}")),
    ] {
        let (status, _, response) = call(
            &app,
            guarded_request(method, &path, None, Some(&config.origin), false, None),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} without a session: {response}"
        );
        assert_eq!(response["code"], "unauthenticated");
    }

    // The correct combination works for every route: options, finish and
    // delete, each with session, Origin and CSRF.
    let (status, _, body) = call(
        &app,
        guarded_request(
            Method::POST,
            PASSKEYS_OPTIONS,
            Some(&browser),
            Some(&config.origin),
            true,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "passkey options: {body}");
    let ceremony_id: Uuid = body["ceremony_id"]
        .as_str()
        .expect("the ceremony id is a string")
        .parse()
        .expect("the ceremony id is a UUID");
    let challenge: CreationChallengeResponse = serde_json::from_value(body["options"].clone())
        .expect("the options must be a creation challenge");
    let mut authenticator = soft_token();
    let credential = authenticator
        .do_registration(origin(&config), challenge)
        .expect("the software authenticator must complete registration");
    let credential = serde_json::to_value(&credential).expect("the credential is JSON");
    let finish = json!({ "ceremony_id": ceremony_id, "credential": credential });

    let (status, _, body) = call(
        &app,
        guarded_request(
            Method::POST,
            PASSKEYS_FINISH,
            Some(&browser),
            Some(&config.origin),
            true,
            Some(&finish),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "passkey finish: {body}");
    let added: Uuid = body["id"]
        .as_str()
        .expect("the new Passkey id is a string")
        .parse()
        .expect("the new Passkey id is a UUID");

    let (status, _, body) = call(
        &app,
        guarded_request(
            Method::DELETE,
            &format!("{PASSKEYS}/{added}"),
            Some(&browser),
            Some(&config.origin),
            true,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "passkey delete: {body}");
    let remaining = passkey_ids(pool).await;
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0], added);

    db.drop().await;
}
