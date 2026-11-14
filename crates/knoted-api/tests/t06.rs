//! T06 acceptance tests: health probes, the request id contract and log
//! redaction.
//!
//! The tests drive the real Axum router through `tower::ServiceExt::oneshot`
//! and use the shared PostgreSQL test database (`TestDb`) so readiness is
//! exercised against applied migrations. A missing database is a test failure,
//! never a skip.

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use knoted_api::build_router;
use knoted_api::logging::{self, LogBuffer};
use knoted_core::config::{LogFormat, LogLevel};
use knoted_core::context::AppContext;
use knoted_core::db::testing::TestDb;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn call(app: axum::Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app
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
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "a JSON response was expected: {error}; body was `{}`",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, headers, json)
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .body(Body::empty())
        .expect("a GET request is well formed")
}

/// UUID carried by the response header, which every response must have.
fn header_request_id(headers: &HeaderMap) -> Uuid {
    let value = headers
        .get("x-request-id")
        .expect("every response must carry X-Request-Id");
    value
        .to_str()
        .expect("X-Request-Id must be ASCII")
        .parse()
        .expect("X-Request-Id must be a UUID")
}

#[tokio::test]
async fn t06_1_pg_down_ready_503_live_200() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("test schema setup failed: {error}; {UNREACHABLE_HINT}"));
    let app = build_router(AppContext::new(db.pool.clone()));

    // With the migrated schema reachable the probe is ready.
    let (status, headers, body) = call(app.clone(), get("/health/ready")).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "ready");
    header_request_id(&headers);

    // Closing the pool simulates PostgreSQL becoming unreachable: readiness
    // must degrade to 503 with the shared error shape, not panic or hang.
    db.pool.close().await;

    let (status, headers, body) = call(app.clone(), get("/health/ready")).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "readiness must fail closed when the database is gone: {body}"
    );
    assert_eq!(body["code"], "dependency_unavailable");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "the error message must explain the failure: {body}"
    );
    assert_eq!(
        body["request_id"],
        header_request_id(&headers).to_string(),
        "the body and header must carry the same request id"
    );

    // Liveness is about the process, not its dependencies.
    let (status, headers, body) = call(app, get("/health/live")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "liveness must stay up without the database: {body}"
    );
    assert_eq!(body["status"], "live");
    header_request_id(&headers);

    db.drop().await;
}

#[tokio::test]
async fn t06_2_error_response_carries_request_id() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("test schema setup failed: {error}; {UNREACHABLE_HINT}"));
    let app = build_router(AppContext::new(db.pool.clone()));

    // Unknown routes answer with the shared error shape and a request id.
    let (status, headers, body) = call(app.clone(), get("/no/such/route")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    assert_eq!(
        body["request_id"],
        header_request_id(&headers).to_string(),
        "404 body and header must share one request id"
    );

    // A well-formed caller-supplied id is echoed unchanged.
    let supplied = Uuid::new_v4();
    let response = app
        .clone()
        .oneshot(
            Request::get("/health/live")
                .header("x-request-id", supplied.to_string())
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("the router must answer");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_request_id(response.headers()),
        supplied,
        "a valid X-Request-Id must be echoed"
    );

    // Anything that is not a UUID is replaced by a generated id.
    let response = app
        .clone()
        .oneshot(
            Request::get("/health/live")
                .header("x-request-id", "not-a-uuid")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("the router must answer");
    let generated = header_request_id(response.headers());
    assert_ne!(generated.to_string(), "not-a-uuid");
    assert!(
        response
            .headers()
            .get("x-request-id")
            .expect("header")
            .to_str()
            .expect("ASCII")
            .parse::<Uuid>()
            .is_ok(),
        "the replacement id must be a UUID"
    );

    // Dependency failures share the same id contract.
    db.pool.close().await;
    let (status, headers, body) = call(app, get("/health/ready")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["request_id"],
        header_request_id(&headers).to_string(),
        "the 503 body and header must share one request id"
    );

    db.drop().await;
}

#[tokio::test]
async fn t06_3_logs_exclude_body_and_cookies() {
    let db = TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("test schema setup failed: {error}; {UNREACHABLE_HINT}"));

    // The subscriber is process-wide, so it is installed once here. Requests
    // from the other tests may interleave, but only this test sends secrets.
    let buffer = LogBuffer::new();
    let sink = buffer.clone();
    logging::init_with_writer(move || sink.clone(), LogLevel::Info, LogFormat::Text)
        .expect("the test logging subscriber must install cleanly");

    let app = build_router(AppContext::new(db.pool.clone()));
    let request = Request::post("/health/live")
        .header(header::COOKIE, "knoted_session=SECRETVALUE")
        .header(header::AUTHORIZATION, "Bearer SECRETVALUE")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"note":"SUPERSECRETBODY"}"#))
        .expect("request");

    let response = app.oneshot(request).await.expect("the router must answer");
    // `/health/live` is GET-only; the rejection is still a logged request that
    // carries a request id.
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let request_id = header_request_id(response.headers());

    let logs = buffer.contents();
    assert!(!logs.is_empty(), "the request must produce a log line");
    assert!(
        logs.contains("request_id"),
        "log lines must record the request id field: {logs}"
    );
    assert!(
        logs.contains(&request_id.to_string()),
        "the logged request id must match the response header: {logs}"
    );
    assert!(
        logs.contains("/health/live"),
        "the request path must be logged: {logs}"
    );
    assert!(
        !logs.contains("SECRETVALUE"),
        "cookies and authorization headers must never be logged: {logs}"
    );
    assert!(
        !logs.contains("SUPERSECRETBODY"),
        "request bodies must never be logged: {logs}"
    );

    db.drop().await;
}
