//! Authenticated HTTP and persistence checks for chat generation.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use knoted_api::build_router_with_auth;
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::AppContext,
    db::testing::TestDb,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Harness {
    db: TestDb,
    ctx: AppContext,
    app: Router,
    cookie: String,
    csrf: String,
}

impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone());
        let mut tx = db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            db,
            app: build_router_with_auth(ctx.clone(), AuthConfig::default()),
            ctx,
            cookie: format!(
                "knoted_session={}; knoted_csrf={}",
                session.token, session.csrf_token
            ),
            csrf: session.csrf_token,
        }
    }

    async fn send(
        &self,
        method: &str,
        endpoint: &str,
        body: Value,
        key: Option<&str>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("/api/v1/chat{endpoint}"))
            .header("cookie", &self.cookie)
            .header("origin", AuthConfig::default().origin)
            .header("x-csrf-token", &self.csrf)
            .header("content-type", "application/json");
        if let Some(key) = key {
            req = req.header("idempotency-key", key);
        }
        let response = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }

    async fn session(&self) -> Uuid {
        let (status, headers, body) = self.send("POST", "/sessions", json!({}), None).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(headers["etag"], "\"1\"");
        assert_eq!(body["provider"], "local");
        body["id"].as_str().unwrap().parse().unwrap()
    }

    async fn submit(&self, session: Uuid, key: &str, content: &str) -> (StatusCode, Value) {
        let (status, _, body) = self
            .send(
                "POST",
                &format!("/sessions/{session}/messages"),
                json!({"content":content}),
                Some(key),
            )
            .await;
        (status, body)
    }
}

#[tokio::test]
async fn t62_1_duplicate_idempotency_key_creates_one_turn_and_run() {
    let h = Harness::new().await;
    let session = h.session().await;
    let (first, replay) = tokio::join!(
        h.submit(session, "same-request", "中文问题"),
        h.submit(session, "same-request", "中文问题")
    );
    assert_eq!(first.0, StatusCode::CREATED, "{}", first.1);
    assert_eq!(first, replay);
    assert_eq!(first.1["run"]["state"], "queued");
    assert_eq!(first.1["message"]["body"], "中文问题");
    assert_eq!(first.1["message"]["role"], "user");
    assert_eq!(
        h.submit(session, "same-request", "另一个问题").await.0,
        StatusCode::CONFLICT
    );

    let (messages, runs, jobs, events): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM chat_messages), (SELECT count(*) FROM generation_runs),
                (SELECT count(*) FROM jobs WHERE kind='chat.generate'),
                (SELECT count(*) FROM chat_run_events WHERE type='queued')",
    )
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    assert_eq!((messages, runs, jobs, events), (2, 1, 1, 1));
    let (status, _, detail) = h
        .send("GET", &format!("/sessions/{session}"), Value::Null, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["messages"][0]["body"], "中文问题");
    assert_eq!(detail["messages"][1]["incomplete"], true);
}

fn run_id(body: &Value) -> Uuid {
    body["run"]["id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn t62_2_cancel_holds_next_run_until_upstream_confirms() {
    use knoted_core::{jobs, rag::chat};
    let h = Harness::new().await;
    let session = h.session().await;
    let first = run_id(&h.submit(session, "first", "问题一").await.1);
    let second = run_id(&h.submit(session, "second", "问题二").await.1);
    let lease = jobs::claim(&h.db.pool, "first-worker")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chat::begin_generation(&h.ctx, &lease, first).await.unwrap(),
        chat::GenerationStart::Fresh
    );
    chat::mark_upstream_requested(&h.ctx, &lease, first, "fixture-model")
        .await
        .unwrap();

    let endpoint = format!("/runs/{first}/cancel");
    let (status, _, response) = h.send("POST", &endpoint, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["state"], "started");
    assert!(
        !chat::confirm_generation_stop(&h.ctx, &lease, first, false)
            .await
            .unwrap()
    );
    assert!(
        jobs::claim(&h.db.pool, "second-worker")
            .await
            .unwrap()
            .is_none()
    );
    let (held, state): (bool, String) =
        sqlx::query_as("SELECT admitted,state FROM generation_runs WHERE id=$1")
            .bind(first)
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert!(held);
    assert_eq!(state, "cancel_requested");

    assert!(
        chat::confirm_generation_stop(&h.ctx, &lease, first, true)
            .await
            .unwrap()
    );
    let next = jobs::claim(&h.db.pool, "second-worker")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chat::begin_generation(&h.ctx, &next, second).await.unwrap(),
        chat::GenerationStart::Fresh
    );
    let (_, _, done) = h
        .send("GET", &format!("/runs/{first}"), Value::Null, None)
        .await;
    assert_eq!(done["state"], "cancelled");
}

#[tokio::test]
async fn t62_3_taking_over_does_not_restart_an_unfinished_generation() {
    use knoted_core::{jobs, rag::chat};
    let h = Harness::new().await;
    let session = h.session().await;
    let response = h.submit(session, "first", "是否完成").await.1;
    let first = run_id(&response);
    let lease = jobs::claim(&h.db.pool, "before-restart")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chat::begin_generation(&h.ctx, &lease, first).await.unwrap(),
        chat::GenerationStart::Fresh
    );
    chat::mark_upstream_requested(&h.ctx, &lease, first, "fixture-model")
        .await
        .unwrap();
    chat::record_delta(&h.ctx, &lease, first, "未完成的文字")
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET lease_expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(lease.job_id)
        .execute(&h.db.pool)
        .await
        .unwrap();

    let recovered = jobs::claim(&h.db.pool, "restart").await.unwrap().unwrap();
    assert_ne!(recovered.token, lease.token);
    assert_eq!(
        chat::begin_generation(&h.ctx, &recovered, first)
            .await
            .unwrap(),
        chat::GenerationStart::Interrupted
    );
    let (_, _, run) = h
        .send("GET", &format!("/runs/{first}"), Value::Null, None)
        .await;
    assert_eq!(run["state"], "failed");
    assert_eq!(run["error"], "generation_interrupted");
    assert_eq!(run["partial"], true);
    let (status, _, _) = h
        .send("POST", &format!("/runs/{first}/retry"), Value::Null, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        chat::confirm_generation_stop(&h.ctx, &recovered, first, true)
            .await
            .unwrap()
    );

    let (status, _, retry) = h
        .send("POST", &format!("/runs/{first}/retry"), Value::Null, None)
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(retry["message_id"], response["message"]["id"]);
    assert_ne!(retry["id"], response["run"]["id"]);
    let events: Vec<(i64, String)> =
        sqlx::query_as("SELECT seq,type FROM chat_run_events WHERE run_id=$1 ORDER BY seq")
            .bind(first)
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        vec![
            (1, "queued".into()),
            (2, "started".into()),
            (3, "delta".into()),
            (4, "failed".into())
        ]
    );
    let (messages, runs): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM chat_messages),(SELECT count(*) FROM generation_runs)",
    )
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    assert_eq!((messages, runs), (2, 2));
}
