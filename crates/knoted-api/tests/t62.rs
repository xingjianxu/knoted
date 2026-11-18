//! Authenticated HTTP and persistence checks for chat generation.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use knoted_api::build_router_with_auth;
use knoted_core::{
    auth::{self, sessions},
    config::{AuthConfig, SearchConfig},
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
        Self::with_search(SearchConfig::default()).await
    }

    async fn with_search(config: SearchConfig) -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone()).with_search(config);
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
        let (status, headers, bytes) = self
            .raw(method, &format!("/chat{endpoint}"), body, key)
            .await;
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }

    async fn raw(
        &self,
        method: &str,
        endpoint: &str,
        body: Value,
        key: Option<&str>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("/api/v1{endpoint}"))
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
        (status, headers, bytes.to_vec())
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

#[tokio::test]
async fn t62_4_completed_reply_is_searchable_and_soft_delete_is_immediate() {
    use knoted_core::{
        jobs::{self, ExecutionOutcome, JobOutcome, JobState},
        rag::chat,
        search::{chunks, meili, service},
    };
    let search = SearchConfig {
        enabled: true,
        base_url: std::env::var("KNOTED_TEST_MEILI_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:15605".into()),
        index_uid: format!("knoted_t62_{}", Uuid::new_v4().simple()),
        ..SearchConfig::default()
    };
    let h = Harness::with_search(search).await;
    let session = h.session().await;
    let first = run_id(&h.submit(session, "first", "请概括资料").await.1);

    let lease = jobs::claim(&h.db.pool, "fixture-generation")
        .await
        .unwrap()
        .unwrap();
    chat::begin_generation(&h.ctx, &lease, first).await.unwrap();
    chat::mark_upstream_requested(&h.ctx, &lease, first, "fixture")
        .await
        .unwrap();
    chat::record_delta(
        &h.ctx,
        &lease,
        first,
        "这篇笔记记录了中文检索与知识管理之间的关系。",
    )
    .await
    .unwrap();
    chat::complete_generation(&h.ctx, &lease, first)
        .await
        .unwrap();
    assert!(
        jobs::finish_with_token(
            &h.db.pool,
            &lease,
            JobOutcome::Succeeded,
            &[JobState::Running]
        )
        .await
        .unwrap()
    );

    let (status, headers, response) = h
        .raw(
            "POST",
            &format!("/chat/runs/{first}/events"),
            Value::Null,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/event-stream");
    let events: Vec<Value> = String::from_utf8(response)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["queued", "started", "delta", "completed"]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["seq"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    let (_, _, detail) = h
        .send("GET", &format!("/sessions/{session}"), Value::Null, None)
        .await;
    let reply = &detail["messages"][1];
    assert_eq!(reply["incomplete"], false);
    assert!(reply["body"].as_str().unwrap().contains("知识管理"));

    assert_eq!(service::project_pending(&h.ctx).await.unwrap(), 2);
    while let Some(lease) = jobs::claim(&h.db.pool, "fixture-publication")
        .await
        .unwrap()
    {
        let job = jobs::load_claimed(&h.db.pool, &lease)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.kind, meili::PUBLISH_JOB_KIND);
        assert_eq!(
            meili::execute(&h.ctx, &lease, &job).await,
            ExecutionOutcome::Succeeded
        );
        assert!(
            jobs::finish_with_token(
                &h.db.pool,
                &lease,
                JobOutcome::Succeeded,
                &[JobState::Running]
            )
            .await
            .unwrap()
        );
    }

    let endpoint = "/search?type=chat&q=%E7%9F%A5%E8%AF%86%E7%AE%A1%E7%90%86";
    let (status, _, body) = h.raw("GET", endpoint, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    let before: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(before["items"].as_array().unwrap().len(), 1);
    assert_eq!(before["items"][0]["id"], reply["id"]);
    assert_eq!(before["items"][0]["session_id"], session.to_string());

    assert_eq!(chunks::enqueue_missing(&h.ctx).await.unwrap(), 0);
    let (chunks, vectors): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM rag_chunks),(SELECT count(*) FROM rag_vectors)",
    )
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    assert_eq!((chunks, vectors), (0, 0));
    let (status, _, _) = h
        .send(
            "DELETE",
            &format!("/sessions/{session}?expected_version={}", detail["version"]),
            Value::Null,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, body) = h.raw("GET", endpoint, Value::Null, None).await;
    assert_eq!(status, StatusCode::OK);
    let after: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(after["items"], json!([]));
}

#[tokio::test]
async fn t62_5_generation_streams_the_selected_provider_to_durable_events() {
    use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
    use knoted_core::{
        jobs::{self, ExecutionOutcome},
        rag::{
            chat,
            providers::{ChatProvidersConfig, ProviderConfig, ProviderRegistry},
        },
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let server = Router::new().route("/v1/chat/completions", post(
        |State(requests):State<Arc<AtomicUsize>>, Json(request):Json<Value>| async move {
            assert_eq!(request["model"],"fixture-model");
            let messages = request["messages"].as_array().unwrap();
            assert_eq!(messages.last().unwrap()["content"],"请检查这句话");
            requests.fetch_add(1,Ordering::Relaxed);
            let lines = [
                json!({"model":"fixture-model","choices":[{"index":0,"delta":{"content":"持久回答"},"finish_reason":null}]}),
                json!({"model":"fixture-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            ];
            let body = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",lines[0],lines[1]);
            ([("content-type","text/event-stream")],body).into_response()
        }
    )).with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let fixture = tempfile::tempdir().unwrap();
    let credential_file = fixture.path().join("credential");
    std::fs::write(&credential_file, "fixture-key").unwrap();
    let registry = ProviderRegistry::new(ChatProvidersConfig {
        local: None,
        external: [(
            "test_provider".into(),
            ProviderConfig {
                base_url,
                model: "fixture-model".into(),
                credential_file,
            },
        )]
        .into(),
    })
    .unwrap();

    let mut h = Harness::new().await;
    h.ctx = h.ctx.with_chat_providers(Arc::new(registry));
    h.app = build_router_with_auth(h.ctx.clone(), AuthConfig::default());
    let (status, _, body) = h
        .send(
            "POST",
            "/sessions",
            json!({"provider":"test_provider"}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let session = body["id"].as_str().unwrap().parse().unwrap();
    let id = run_id(&h.submit(session, "probe", "请检查这句话").await.1);
    let lease = jobs::claim(&h.db.pool, "live-handler")
        .await
        .unwrap()
        .unwrap();
    let job = jobs::load_claimed(&h.db.pool, &lease)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chat::execute_generation(&h.ctx, &lease, &job).await,
        ExecutionOutcome::Succeeded
    );
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    let (_, _, detail) = h
        .send("GET", &format!("/sessions/{session}"), Value::Null, None)
        .await;
    assert_eq!(detail["messages"][1]["body"], "持久回答");
    let (_, _, run) = h
        .send("GET", &format!("/runs/{id}"), Value::Null, None)
        .await;
    assert_eq!(run["state"], "completed");
    handle.abort();
}

#[tokio::test]
async fn t62_6_worker_recovers_a_terminal_job_with_an_unsettled_run() {
    use knoted_core::{
        jobs::{self, JobOutcome, JobState},
        rag::chat,
    };
    let h = Harness::new().await;
    let session = h.session().await;
    let first = run_id(&h.submit(session, "first", "请求一").await.1);
    let next = run_id(&h.submit(session, "second", "请求二").await.1);
    let lease = jobs::claim(&h.db.pool, "worker-before-db-failure")
        .await
        .unwrap()
        .unwrap();
    chat::begin_generation(&h.ctx, &lease, first).await.unwrap();
    assert!(
        jobs::finish_with_token(
            &h.db.pool,
            &lease,
            JobOutcome::Failed {
                error: "fixture".into()
            },
            &[JobState::Running],
        )
        .await
        .unwrap()
    );
    assert!(jobs::claim(&h.db.pool, "recover").await.unwrap().is_none());
    assert!(chat::recover_terminal_generation(&h.ctx).await.unwrap());
    let next_lease = jobs::claim(&h.db.pool, "recover").await.unwrap().unwrap();
    assert_eq!(
        chat::begin_generation(&h.ctx, &next_lease, next)
            .await
            .unwrap(),
        chat::GenerationStart::Fresh
    );
    let (_, _, first) = h
        .send("GET", &format!("/runs/{first}"), Value::Null, None)
        .await;
    assert_eq!(first["state"], "failed");
    assert_eq!(first["error"], "generation_interrupted");
}
