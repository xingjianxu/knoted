//! T14 acceptance tests: Passkey registration and login.
//!
//! The tests run against the real local PostgreSQL test database with a
//! per-test schema, drive the production router with a fixed clock, and use
//! `webauthn-authenticator-rs`'s `SoftToken` to produce genuine registration and
//! assertion responses. Every assertion therefore passes through the real
//! `webauthn-rs` cryptography: challenge, origin, RP ID hash, signature and
//! counter. A missing database is a failure, never a skip.
//!
//! Covered here: replay rejection after a successful registration, a tampered
//! `clientDataJSON` origin and deliberately wrong server origin/RP ID settings,
//! the full login flow with counter/last-used metadata, single-use bootstrap
//! tokens, and the T13-review guard that a stale bootstrap token cannot add a
//! Passkey to an account that already has one.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use knoted_api::build_router_with_auth;
use knoted_core::auth::{self, TokenPurpose};
use knoted_core::config::AuthConfig;
use knoted_core::context::{AppContext, FixedClock};
use knoted_core::db::testing::TestDb;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use webauthn_authenticator_rs::AuthenticatorBackend;
use webauthn_authenticator_rs::prelude::{
    CreationChallengeResponse, RegisterPublicKeyCredential, RequestChallengeResponse, Url,
    WebauthnAuthenticator,
};
use webauthn_authenticator_rs::softtoken::SoftToken;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

const BOOTSTRAP_OPTIONS: &str = "/api/v1/auth/bootstrap/options";
const BOOTSTRAP_FINISH: &str = "/api/v1/auth/bootstrap/finish";
const LOGIN_OPTIONS: &str = "/api/v1/auth/login/options";
const LOGIN_FINISH: &str = "/api/v1/auth/login/finish";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 5, 6, 7, 8)
        .single()
        .expect("the fixed instant is valid")
}

fn context(pool: &PgPool, now: DateTime<Utc>) -> AppContext {
    AppContext::new(pool.clone()).with_clock(Arc::new(FixedClock::new(now)))
}

fn config() -> AuthConfig {
    AuthConfig::default()
}

/// A SoftToken with `falsify_uv = true`: the passkey flows require user
/// verification and this fake authenticator reports it.
fn soft_token() -> SoftToken {
    SoftToken::new(true)
        .expect("the software authenticator must initialise")
        .0
}

fn origin(config: &AuthConfig) -> Url {
    Url::parse(&config.origin).expect("the configured origin is a valid URL")
}

fn json_request(path: &str, body: &Value, origin: &str) -> Request<Body> {
    Request::post(path)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "t14 test agent")
        .body(Body::from(
            serde_json::to_vec(body).expect("the test body is JSON"),
        ))
        .expect("the request is well formed")
}

fn empty_request(path: &str, origin: &str) -> Request<Body> {
    Request::post(path)
        .header(header::ORIGIN, origin)
        .header(header::USER_AGENT, "t14 test agent")
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

async fn issue_bootstrap(pool: &PgPool, now: DateTime<Utc>) -> String {
    auth::issue_token(pool, TokenPurpose::Bootstrap, now)
        .await
        .expect("a bootstrap token must be issued")
        .plaintext()
        .to_owned()
}

/// Starts a registration ceremony and returns its id and browser options.
async fn bootstrap_options(app: &Router, config: &AuthConfig, token: &str) -> (Uuid, Value) {
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
    let ceremony_id = body["ceremony_id"]
        .as_str()
        .expect("the response must carry a ceremony id")
        .parse()
        .expect("the ceremony id must be a UUID");
    (ceremony_id, body["options"].clone())
}

/// Produces a real registration response for the challenge.
fn register(
    token: &mut SoftToken,
    config: &AuthConfig,
    options: &Value,
) -> (Value, RegisterPublicKeyCredential) {
    let challenge: CreationChallengeResponse = serde_json::from_value(options.clone())
        .expect("the server options must deserialize as a creation challenge");
    let credential = token
        .do_registration(origin(config), challenge)
        .expect("the software authenticator must complete registration");
    (
        serde_json::to_value(&credential).expect("the credential is JSON"),
        credential,
    )
}

async fn bootstrap_finish(
    app: &Router,
    config: &AuthConfig,
    ceremony_id: Uuid,
    credential: &Value,
    name: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let mut body = json!({ "ceremony_id": ceremony_id, "credential": credential });
    if let Some(name) = name {
        body["name"] = json!(name);
    }
    call(app, json_request(BOOTSTRAP_FINISH, &body, &config.origin)).await
}

async fn login_options(app: &Router, config: &AuthConfig) -> (Uuid, Value) {
    let (status, _, body) = call(app, empty_request(LOGIN_OPTIONS, &config.origin)).await;
    assert_eq!(status, StatusCode::OK, "login options failed: {body}");
    let ceremony_id = body["ceremony_id"]
        .as_str()
        .expect("the response must carry a ceremony id")
        .parse()
        .expect("the ceremony id must be a UUID");
    (ceremony_id, body["options"].clone())
}

async fn login_finish(
    app: &Router,
    config: &AuthConfig,
    ceremony_id: Uuid,
    credential: &Value,
) -> (StatusCode, HeaderMap, Value) {
    let body = json!({ "ceremony_id": ceremony_id, "credential": credential });
    call(app, json_request(LOGIN_FINISH, &body, &config.origin)).await
}

/// Registers the owner and returns the login-ready session response.
async fn bootstrap_owner(
    app: &Router,
    config: &AuthConfig,
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Value {
    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(app, config, &token).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, config, &options);
    let (status, _, body) =
        bootstrap_finish(app, config, ceremony_id, &credential, Some("Test Passkey")).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bootstrap finish failed: {body}"
    );
    body
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    let (count,): (i64,) = sqlx::query_as(sql)
        .fetch_one(pool)
        .await
        .expect("the count query must succeed");
    count
}

async fn passkey_count(pool: &PgPool) -> i64 {
    count(pool, "SELECT count(*) FROM passkey_credentials").await
}

async fn session_count(pool: &PgPool) -> i64 {
    count(pool, "SELECT count(*) FROM sessions").await
}

async fn user_count(pool: &PgPool) -> i64 {
    count(pool, "SELECT count(*) FROM users").await
}

/// Stored Passkey JSON and its `last_used_at`.
async fn stored_passkey(pool: &PgPool) -> (Value, Option<DateTime<Utc>>) {
    sqlx::query_as("SELECT passkey, last_used_at FROM passkey_credentials")
        .fetch_one(pool)
        .await
        .expect("the passkey row must exist")
}

fn counter(value: &Value) -> u64 {
    value["cred"]["counter"]
        .as_u64()
        .unwrap_or_else(|| panic!("the stored passkey must carry a counter: {value}"))
}

fn set_cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn t14_1_replayed_challenge_rejected() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, &config, &options);

    let (status, headers, body) =
        bootstrap_finish(&app, &config, ceremony_id, &credential, None).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(body["session"]["current"], true);
    assert!(
        body["csrf_token"]
            .as_str()
            .is_some_and(|token| token.len() >= 16),
        "a CSRF token must be returned: {body}"
    );
    let cookies = set_cookies(&headers);
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.starts_with("knoted_session=")
                && cookie.contains("HttpOnly")
                && cookie.contains("Secure")
                && cookie.contains("SameSite=Lax")),
        "the session cookie must carry its security attributes: {cookies:?}"
    );
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.starts_with("knoted_csrf=")),
        "the readable CSRF cookie must be set: {cookies:?}"
    );

    // The exact same ceremony and credential cannot be submitted again.
    let (status, _, body) = bootstrap_finish(&app, &config, ceremony_id, &credential, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "ceremony_replayed");

    // An unknown ceremony id is a 409 too: the contract declares no 404 for
    // these endpoints.
    let (status, _, body) =
        bootstrap_finish(&app, &config, Uuid::new_v4(), &credential, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "ceremony_not_found");

    assert_eq!(passkey_count(pool).await, 1, "no duplicate credential");
    assert_eq!(session_count(pool).await, 1, "no second session");
    assert_eq!(user_count(pool).await, 1, "still exactly one owner");

    db.drop().await;
}

#[tokio::test]
async fn t14_2_wrong_origin_or_rp_id_rejected() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let ctx = context(pool, now);
    let app = build_router_with_auth(ctx.clone(), config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (credential_json, credential) = register(&mut authenticator, &config, &options);

    // 1. A genuinely signed response whose `clientDataJSON` says
    //    `https://evil.example`. The low-level authenticator API is used on
    //    purpose: it signs the client data without applying the browser origin
    //    guard, so the signature is valid and only the server's exact-origin
    //    check can reject the credential.
    let challenge: CreationChallengeResponse = serde_json::from_value(options.clone())
        .expect("the server options must deserialize as a creation challenge");
    let evil_origin = authenticator
        .perform_register(
            Url::parse("https://evil.example").expect("the evil origin is a URL"),
            challenge.public_key,
            60_000,
        )
        .expect("the low-level authenticator must sign the evil origin");
    let evil_json = serde_json::to_value(&evil_origin).expect("the credential is JSON");
    let (status, _, body) = bootstrap_finish(&app, &config, ceremony_id, &evil_json, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "credential_rejected");
    assert_eq!(passkey_count(pool).await, 0, "no credential may be stored");
    assert_eq!(session_count(pool).await, 0, "no session may be created");

    // 2. Tamper with an already-signed `clientDataJSON` instead: the origin
    //    changes without a matching signature, so signature integrity rejects
    //    it as well.
    let mut tampered = credential.clone();
    let mut client_data: Value = serde_json::from_slice(&tampered.response.client_data_json)
        .expect("the client data must be JSON");
    client_data["origin"] = json!("https://evil.example");
    tampered.response.client_data_json =
        serde_json::to_vec(&client_data).expect("the tampered client data is JSON");
    let tampered_json = serde_json::to_value(&tampered).expect("the tampered credential is JSON");

    let (status, _, body) =
        bootstrap_finish(&app, &config, ceremony_id, &tampered_json, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "credential_rejected");
    assert_eq!(passkey_count(pool).await, 0);
    assert_eq!(session_count(pool).await, 0);

    // 3. A server configured with the wrong origin rejects the genuine
    //    credential, because the signed origin no longer matches.
    let wrong_origin = AuthConfig {
        origin: "https://wlab.132140.xyz:15602".to_owned(),
        ..config.clone()
    };
    let wrong_origin_app = build_router_with_auth(ctx.clone(), wrong_origin.clone());
    let (status, _, body) = bootstrap_finish(
        &wrong_origin_app,
        &wrong_origin,
        ceremony_id,
        &credential_json,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "credential_rejected");
    assert_eq!(passkey_count(pool).await, 0);
    assert_eq!(session_count(pool).await, 0);

    // 4. A server configured with a parent RP ID rejects the credential whose
    //    authenticator data hashes the original RP ID.
    let wrong_rp_id = AuthConfig {
        rp_id: "132140.xyz".to_owned(),
        ..config.clone()
    };
    let wrong_rp_id_app = build_router_with_auth(ctx, wrong_rp_id.clone());
    let (status, _, body) = bootstrap_finish(
        &wrong_rp_id_app,
        &wrong_rp_id,
        ceremony_id,
        &credential_json,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body: {body}");
    assert_eq!(body["code"], "credential_rejected");
    assert_eq!(passkey_count(pool).await, 0);
    assert_eq!(session_count(pool).await, 0);

    // None of the rejected attempts consumed the ceremony, so the genuine
    // credential still registers once with the correct configuration.
    let (status, _, body) =
        bootstrap_finish(&app, &config, ceremony_id, &credential_json, None).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(passkey_count(pool).await, 1);
    assert_eq!(session_count(pool).await, 1);

    db.drop().await;
}

#[tokio::test]
async fn t14_3_login_flow_and_metadata() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());
    let mut authenticator = soft_token();

    // Register first, using the same SoftToken so it holds the private key.
    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let (credential, _) = register(&mut authenticator, &config, &options);
    let (status, _, body) =
        bootstrap_finish(&app, &config, ceremony_id, &credential, Some("Laptop")).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let owner_id = body["session"]["user_id"].clone();

    let (before, last_used_before) = stored_passkey(pool).await;
    assert_eq!(last_used_before, None, "registration is not a login");
    let before_counter = counter(&before);

    // Authenticate with a real assertion generated from the login challenge.
    let (login_ceremony, login_options_json) = login_options(&app, &config).await;
    let challenge: RequestChallengeResponse =
        serde_json::from_value(login_options_json).expect("the login options are a challenge");
    let assertion = authenticator
        .do_authentication(origin(&config), challenge)
        .expect("the software authenticator must sign the challenge");
    let assertion_json = serde_json::to_value(&assertion).expect("the assertion is JSON");

    let (status, headers, body) =
        login_finish(&app, &config, login_ceremony, &assertion_json).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["session"]["user_id"], owner_id);
    assert_eq!(body["session"]["current"], true);
    assert!(
        body["csrf_token"]
            .as_str()
            .is_some_and(|token| token.len() >= 16)
    );
    let cookies = set_cookies(&headers);
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.starts_with("knoted_session=") && cookie.contains("HttpOnly")),
        "login must set the session cookie: {cookies:?}"
    );

    // The assertion updated the credential metadata and recorded the login.
    let (after, last_used_after) = stored_passkey(pool).await;
    assert!(
        counter(&after) > before_counter,
        "the counter must advance: before {before_counter}, after {}",
        counter(&after)
    );
    assert_eq!(
        last_used_after,
        Some(now),
        "last_used_at must record the login"
    );

    let (verified,): (Option<DateTime<Utc>>,) =
        sqlx::query_as("SELECT last_verified_at FROM sessions WHERE id = $1")
            .bind(
                body["session"]["id"]
                    .as_str()
                    .expect("the session id is a string")
                    .parse::<Uuid>()
                    .expect("the session id is a UUID"),
            )
            .fetch_one(pool)
            .await
            .expect("the login session must exist");
    assert_eq!(
        verified,
        Some(now),
        "a login ceremony creates a verified session"
    );

    // A consumed login ceremony cannot be replayed either.
    let sessions_before = session_count(pool).await;
    let (status, _, body) = login_finish(&app, &config, login_ceremony, &assertion_json).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "ceremony_replayed");
    assert_eq!(
        session_count(pool).await,
        sessions_before,
        "a replay must not mint another session"
    );

    // An unknown login ceremony is a 409 as well, never an undeclared 404.
    let (status, _, body) = login_finish(&app, &config, Uuid::new_v4(), &assertion_json).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "ceremony_not_found");

    db.drop().await;
}

#[tokio::test]
async fn t14_4_bootstrap_token_single_use() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, &config, &options);
    let (status, _, body) = bootstrap_finish(&app, &config, ceremony_id, &credential, None).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    assert_eq!(user_count(pool).await, 1);
    assert_eq!(passkey_count(pool).await, 1);

    // The same token cannot start a second ceremony after it was consumed.
    let (status, _, body) = call(
        &app,
        json_request(
            BOOTSTRAP_OPTIONS,
            &json!({ "bootstrap_token": token }),
            &config.origin,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "consumed_token");
    assert_eq!(user_count(pool).await, 1, "no second owner");
    assert_eq!(passkey_count(pool).await, 1, "no second Passkey");
    assert_eq!(session_count(pool).await, 1, "no second session");

    db.drop().await;
}

/// The T13-review account-takeover guard, exercised with a token that is
/// genuine and still live after the owner already has a Passkey.
#[tokio::test]
async fn t14_5_bootstrap_refused_when_owner_has_passkey() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());

    bootstrap_owner(&app, &config, pool, now).await;
    assert_eq!(passkey_count(pool).await, 1);
    assert_eq!(session_count(pool).await, 1);

    // A token issued before the owner got its Passkey is still valid in the
    // database (the CLI would normally refuse to issue one, so insert the row
    // directly to model that stale token).
    let plaintext = "stale-bootstrap-token-for-t14-5";
    let hash = auth::hash_token(plaintext);
    sqlx::query(
        "INSERT INTO credential_tokens (id, purpose, token_hash, created_at, expires_at) \
         VALUES ($1, 'bootstrap', $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(hash.as_slice())
    .bind(now)
    .bind(now + chrono::Duration::minutes(10))
    .execute(pool)
    .await
    .expect("the stale token must be stored");

    let (ceremony_id, options) = bootstrap_options(&app, &config, plaintext).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, &config, &options);
    let (status, _, body) = bootstrap_finish(&app, &config, ceremony_id, &credential, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "bootstrap_already_completed");

    assert_eq!(
        passkey_count(pool).await,
        1,
        "a stale token must not add a second Passkey"
    );
    assert_eq!(user_count(pool).await, 1);
    assert_eq!(session_count(pool).await, 1, "no session may be created");
    let consumed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT consumed_at FROM credential_tokens WHERE token_hash = $1")
            .bind(hash.as_slice())
            .fetch_one(pool)
            .await
            .expect("the stale token row must still exist");
    assert_eq!(
        consumed, None,
        "the refused finish must roll the token consumption back"
    );

    db.drop().await;
}

/// The challenge is valid for five minutes and no longer.
#[tokio::test]
async fn t14_6_ceremony_expires_after_five_minutes() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let clock = Arc::new(FixedClock::new(now));
    let ctx = AppContext::new(pool.clone()).with_clock(clock.clone());
    let app = build_router_with_auth(ctx, config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, &config, &options);

    clock.advance(Duration::minutes(5));
    let (status, _, body) = bootstrap_finish(&app, &config, ceremony_id, &credential, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "ceremony_expired");
    assert_eq!(passkey_count(pool).await, 0);
    assert_eq!(session_count(pool).await, 0);
    let consumed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT consumed_at FROM credential_tokens WHERE token_hash = $1")
            .bind(auth::hash_token(&token).as_slice())
            .fetch_one(pool)
            .await
            .expect("the bootstrap token row must exist");
    assert_eq!(
        consumed, None,
        "an expired ceremony must not burn the token"
    );

    db.drop().await;
}

/// Transport hints are filtered to the contract's `PasskeyTransport` enum.
#[tokio::test]
async fn t14_7_unknown_transports_are_dropped() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (mut credential_json, _) = register(&mut authenticator, &config, &options);

    // The attestation object is unchanged; transports are the browser's hint
    // and not part of the signed authenticator data. Adding the non-contract
    // `test`/`unknown` values must not make the registration fail.
    credential_json["response"]["transports"] =
        json!(["usb", "test", "unknown", "internal", "hybrid"]);
    let (status, _, body) =
        bootstrap_finish(&app, &config, ceremony_id, &credential_json, None).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");

    let transports: Vec<String> = sqlx::query_scalar("SELECT transports FROM passkey_credentials")
        .fetch_one(pool)
        .await
        .expect("the passkey row must exist");
    assert_eq!(transports, vec!["usb", "internal", "hybrid"]);

    db.drop().await;
}

/// Two concurrent finishes with the same ceremony and credential: exactly one
/// wins, and only one Passkey and one session exist afterwards.
#[tokio::test]
async fn t14_8_concurrent_finish_is_single_use() {
    let db = test_db().await;
    let pool = &db.pool;
    let now = fixed_now();
    let config = config();
    let app = build_router_with_auth(context(pool, now), config.clone());

    let token = issue_bootstrap(pool, now).await;
    let (ceremony_id, options) = bootstrap_options(&app, &config, &token).await;
    let mut authenticator = soft_token();
    let (credential, _) = register(&mut authenticator, &config, &options);

    let (first, second) = tokio::join!(
        bootstrap_finish(&app, &config, ceremony_id, &credential, None),
        bootstrap_finish(&app, &config, ceremony_id, &credential, None),
    );
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one finish must succeed: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1,
        "exactly one finish must be rejected: {statuses:?}"
    );
    let loser = if first.0 == StatusCode::CONFLICT {
        first.2
    } else {
        second.2
    };
    assert_eq!(loser["code"], "ceremony_replayed");

    assert_eq!(passkey_count(pool).await, 1, "only one Passkey may exist");
    assert_eq!(session_count(pool).await, 1, "only one session may exist");
    assert_eq!(user_count(pool).await, 1, "only one owner may exist");

    db.drop().await;
}
