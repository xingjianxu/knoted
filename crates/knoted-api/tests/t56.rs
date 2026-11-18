use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router;
use knoted_core::{
    auth::{self, sessions},
    config::{AuthConfig, FilesConfig},
    context::AppContext,
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, FinishExecution, Job, JobSpec},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    _db: TestDb,
    ctx: AppContext,
    app: Router,
    cookie: String,
    csrf: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
            enabled: true,
            ..Default::default()
        });
        let mut tx = ctx.pool().begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            _db: db,
            app: build_router(ctx.clone()),
            ctx,
            cookie: format!(
                "knoted_session={}; knoted_csrf={}",
                session.token, session.csrf_token
            ),
            csrf: session.csrf_token,
        }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        signed: bool,
        key: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if signed || method == "POST" {
            request = request.header("cookie", &self.cookie);
        }
        if signed && method == "POST" {
            request = request
                .header("origin", AuthConfig::default().origin)
                .header("x-csrf-token", &self.csrf);
        }
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn enqueue(&self, kind: &str) -> Job {
        let mut tx = self.ctx.pool().begin().await.unwrap();
        let mut spec = JobSpec::new(kind);
        spec.payload = json!({ "private_fixture": "fixture document data" });
        let job = jobs::enqueue(&mut tx, spec).await.unwrap().job;
        tx.commit().await.unwrap();
        job
    }

    async fn finish(&self, lease: &jobs::Lease, outcome: ExecutionOutcome, confirmed: bool) -> Job {
        jobs::service::finish_execution(
            &self.ctx,
            FinishExecution {
                job_id: lease.job_id,
                lease_token: lease.token,
                outcome,
                cancel_confirmed: confirmed,
            },
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn t56_permanent_failure_is_persisted_and_manual_retry_creates_a_new_job() {
    let f = Fixture::new().await;
    let original = f.enqueue("file.extract").await;
    let lease = jobs::claim(f.ctx.pool(), "t56").await.unwrap().unwrap();
    f.finish(
        &lease,
        ExecutionOutcome::PermanentFailure {
            message: "unsupported_format".into(),
        },
        false,
    )
    .await;
    assert!(jobs::claim(f.ctx.pool(), "t56").await.unwrap().is_none());

    let (status, listed) = f
        .call("GET", "/api/v1/jobs?state=failed", true, None, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    let item = &listed["items"][0];
    assert_eq!(item["id"], json!(original.id));
    assert_eq!(item["error"], "unsupported_format");
    assert!(item["started_at"].is_string() && item["finished_at"].is_string());
    assert!(item.get("payload").is_none() && item.get("lease_token").is_none());

    let path = format!("/api/v1/jobs/{}/retry", original.id);
    let (status, _) = f.call("POST", &path, false, None, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, retry) = f.call("POST", &path, true, None, Value::Null).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(retry["id"], json!(original.id));
    assert_eq!(retry["state"], "queued");
    assert!(retry["started_at"].is_null());
    let persisted: (Option<Uuid>, String) =
        sqlx::query_as("SELECT retry_of, state FROM jobs WHERE id=$1")
            .bind(Uuid::parse_str(retry["id"].as_str().unwrap()).unwrap())
            .fetch_one(f.ctx.pool())
            .await
            .unwrap();
    assert_eq!(persisted, (Some(original.id), "queued".into()));
    let (status, listed) = f
        .call("GET", "/api/v1/jobs?state=failed", true, None, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["items"][0]["id"], json!(original.id));
}

#[tokio::test]
async fn t56_cancel_holds_the_lease_until_worker_confirms_and_terminal_requests_conflict() {
    let f = Fixture::new().await;
    let job = f.enqueue("file.extract").await;
    let lease = jobs::claim(f.ctx.pool(), "t56").await.unwrap().unwrap();
    let path = format!("/api/v1/jobs/{}/cancel", job.id);
    let (status, cancelling) = f.call("POST", &path, true, None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelling["state"], "cancel_requested");
    assert!(cancelling["finished_at"].is_null());
    let waiting = f.finish(&lease, ExecutionOutcome::Cancelled, false).await;
    assert_eq!(waiting.state, "cancel_requested");
    assert_eq!(waiting.lease_token, Some(lease.token));
    let finished = f.finish(&lease, ExecutionOutcome::Cancelled, true).await;
    assert_eq!(finished.state, "cancelled");
    assert!(finished.lease_token.is_none());
    let (status, _) = f.call("POST", &path, true, None, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn t56_list_cursor_is_bound_to_the_filter_and_internal_errors_are_redacted() {
    let f = Fixture::new().await;
    let failed = f.enqueue("file.extract").await;
    let lease = jobs::claim(f.ctx.pool(), "t56").await.unwrap().unwrap();
    f.finish(
        &lease,
        ExecutionOutcome::PermanentFailure {
            message: "fixture document data".into(),
        },
        false,
    )
    .await;
    let queued = f.enqueue("fulltext.publish").await;
    let (status, _) = f
        .call("GET", "/api/v1/jobs", false, None, Value::Null)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, latest) = f
        .call("GET", "/api/v1/jobs?limit=1", true, None, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(latest["items"][0]["id"], json!(queued.id));
    let cursor = latest["next_cursor"].as_str().unwrap();
    let (status, _) = f
        .call(
            "GET",
            &format!("/api/v1/jobs?cursor={cursor}&state=failed"),
            true,
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, previous) = f
        .call(
            "GET",
            &format!("/api/v1/jobs?cursor={cursor}"),
            true,
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(previous["items"][0]["id"], json!(failed.id));
    assert_eq!(previous["items"][0]["error"], "job_failed");
    assert!(!previous.to_string().contains("fixture document data"));
}

#[tokio::test]
async fn t56_manual_scan_replays_the_key_and_respects_the_pending_scan_lock() {
    let f = Fixture::new().await;
    const SCAN: &str = "/api/v1/jobs/scan";
    let (status, _) = f.call("POST", SCAN, true, None, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, job) = f
        .call("POST", SCAN, true, Some("scan-key"), json!({"force":true}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(job["kind"], "file.scan");
    let (status, duplicate) = f
        .call("POST", SCAN, true, Some("scan-key"), json!({"force":true}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(duplicate["id"], job["id"]);
    let (status, mismatch) = f
        .call("POST", SCAN, true, Some("scan-key"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(mismatch["code"], "idempotency_conflict");
    let (status, busy) = f.call("POST", SCAN, true, Some("other"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(busy["code"], "scan_pending");
    assert!(
        !knoted_core::files::scanner::enqueue_scan(&f.ctx)
            .await
            .unwrap()
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='job.enqueued.file.scan'")
            .fetch_one(f.ctx.pool())
            .await
            .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn t56_anonymous_jobs_list_has_the_session_guard() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused")
        .unwrap();
    let response = build_router(AppContext::new(pool))
        .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t56_scan_key_expires_after_seven_days_and_replay_keeps_the_original_result() {
    let f = Fixture::new().await;
    let (_, job) = f
        .call("POST", "/api/v1/jobs/scan", true, Some("key"), json!({}))
        .await;
    let path = format!("/api/v1/jobs/{}/cancel", job["id"].as_str().unwrap());
    let (status, cancelled) = f.call("POST", &path, true, None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["state"], "cancelled");
    let (status, replayed) = f
        .call("POST", "/api/v1/jobs/scan", true, Some("key"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(replayed, job);

    sqlx::query("UPDATE jobs SET created_at=now()-interval '8 days' WHERE id=$1")
        .bind(Uuid::parse_str(job["id"].as_str().unwrap()).unwrap())
        .execute(f.ctx.pool())
        .await
        .unwrap();
    let (status, fresh) = f
        .call(
            "POST",
            "/api/v1/jobs/scan",
            true,
            Some("key"),
            json!({"force":true}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(fresh["id"], job["id"]);

    let (status, busy) = f
        .call(
            "POST",
            &format!("/api/v1/jobs/{}/retry", job["id"].as_str().unwrap()),
            true,
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(busy["code"], "scan_pending");
}
