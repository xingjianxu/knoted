//! Real HTTP and PostgreSQL checks for final task state and IANA dates.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use knoted_api::build_router_with_auth;
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::{AppContext, FixedClock},
    db::testing::TestDb,
};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Harness {
    db: TestDb,
    app: Router,
    cookie: String,
    csrf: String,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let clock = FixedClock::new("2026-11-17T00:00:00Z".parse().unwrap());
        let ctx = AppContext::new(db.pool.clone()).with_clock(std::sync::Arc::new(clock.clone()));
        let mut tx = db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            app: build_router_with_auth(ctx, AuthConfig::default()),
            cookie: format!(
                "knoted_session={}; knoted_csrf={}",
                session.token, session.csrf_token
            ),
            csrf: session.csrf_token,
            db,
        }
    }
    async fn send(&self, method: &str, path: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", &self.cookie)
            .header("origin", AuthConfig::default().origin)
            .header("x-csrf-token", &self.csrf)
            .header("content-type", "application/json")
            .body(if body.is_null() {
                Body::empty()
            } else {
                Body::from(serde_json::to_vec(&body).unwrap())
            })
            .unwrap();
        let r = self.app.clone().oneshot(req).await.unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let bytes = to_bytes(r.into_body(), 5 * 1024 * 1024).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, headers, body)
    }
}

#[tokio::test]
async fn t41_dashboard_empty_and_anonymous() {
    let h = Harness::new().await;
    let (s, _, b) = h.send("GET", "/api/v1/dashboard", Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["counts"],
        json!({"notes":0,"tasks_open":0,"tasks_due_today":0,"projects_active":0,"files":0})
    );
    assert_eq!(b["recent_notes"], json!([]));
    assert_eq!(b["due_tasks"], json!([]));
    assert_eq!(b["job_failures"], 0);
    assert!(b.get("indexing_enabled").is_none());
    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    h.db.drop().await;
}

#[tokio::test]
async fn t41_http_completion_disappears_from_due_and_errors_are_safe() {
    let h = Harness::new().await;
    let (status,_,task)=h.send("POST","/api/v1/tasks",json!({"title":"private title","due":{"date":"2020-01-01","time":null,"timezone":"Asia/Shanghai"}})).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let (_, _, before) = h.send("GET", "/api/v1/dashboard", Value::Null).await;
    assert_eq!(before["due_tasks"][0]["id"], task["id"]);
    let (status, _, _) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", task["id"].as_str().unwrap()),
            json!({"expected_version":1,"status":"done"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, after) = h.send("GET", "/api/v1/dashboard", Value::Null).await;
    assert_eq!(after["due_tasks"], json!([]));
    assert_eq!(after["counts"]["tasks_open"], 0);
    assert_eq!(after["tasks_by_status"]["done"], 1);
    // Only this throw-away test schema is affected; session validation survives.
    sqlx::query("ALTER TABLE jobs RENAME TO private_diagnostics_jobs")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (status, _, error) = h.send("GET", "/api/v1/dashboard", Value::Null).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["code"], "dependency_unavailable");
    let text = error.to_string();
    for secret in [
        "private title",
        "private_diagnostics_jobs",
        "SELECT",
        "sqlx",
        "relation",
    ] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(error["request_id"].is_string());
    h.db.drop().await;
}

#[tokio::test]
async fn t41_http_valid_precise_deadline_does_not_require_valid_midnight() {
    let h = Harness::new().await;
    let (status, _, task) = h.send("POST", "/api/v1/tasks", json!({"title":"valid historical deadline","due":{"date":"1919-04-13","time":"03:00","timezone":"Asia/Shanghai"}})).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let (status, _, s) = h.send("GET", "/api/v1/dashboard", Value::Null).await;
    h.db.drop().await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["due_tasks"][0]["id"], task["id"]);
    assert_eq!(s["counts"]["tasks_open"], 1);
}
