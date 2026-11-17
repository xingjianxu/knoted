//! Snapshot HTTP contract against real PostgreSQL; no persistence doubles.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use knoted_api::build_router_with_auth;
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::AppContext,
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
        let ctx = AppContext::new(db.pool.clone());
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
        let (status, headers) = (r.status(), r.headers().clone());
        let bytes = to_bytes(r.into_body(), 5 * 1024 * 1024).await.unwrap();
        (
            status,
            headers,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
        )
    }
    async fn create(&self, title: &str) -> String {
        let r = self
            .send(
                "POST",
                "/api/v1/notes",
                json!({"title":title,"document":{"blocks":[]}}),
            )
            .await;
        assert_eq!(r.0, StatusCode::CREATED, "{}", r.2);
        format!("/api/v1/notes/{}", r.2["id"].as_str().unwrap())
    }
    async fn flush(&self, p: &str) -> Value {
        let r = self
            .send("POST", &format!("{p}/snapshots"), Value::Null)
            .await;
        assert_eq!(r.0, StatusCode::CREATED, "{}", r.2);
        r.2
    }
    async fn update(&self, p: &str, v: i64, title: &str) {
        assert_eq!(
            self.send("PATCH", p, json!({"expected_version":v,"title":title}))
                .await
                .0,
            StatusCode::OK
        );
    }
    async fn restore(&self, p: &str, s: &Value, v: i64) -> (StatusCode, HeaderMap, Value) {
        self.send(
            "POST",
            &format!(
                "{p}/snapshots/{}/restore",
                s["snapshot_id"].as_str().unwrap()
            ),
            json!({"expected_version":v}),
        )
        .await
    }
    async fn state(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('notes',(SELECT jsonb_agg(to_jsonb(n) ORDER BY id) FROM notes n),'history',(SELECT jsonb_agg(to_jsonb(s) ORDER BY snapshot_id) FROM note_snapshots s),'events',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM outbox o))").fetch_one(&self.db.pool).await.unwrap()
    }
}

// A flush must use note/version identity rather than allocating on every call.
#[tokio::test]
async fn t25_1_http_flush_is_concurrently_idempotent_and_versions_are_separate() {
    let h = Harness::new().await;
    let p = h.create("first").await;
    let (a, b) = tokio::join!(h.flush(&p), h.flush(&p));
    assert_eq!(a, b);
    assert_eq!(a["content_version"], 1);
    assert_eq!(a["title"], "first");
    assert_eq!(a["document"], json!({"blocks":[]}));
    assert_eq!(a["reason"], "flush");
    h.update(&p, 1, "second").await;
    h.update(&p, 2, "third").await;
    let c = h.flush(&p).await;
    assert_eq!(c["content_version"], 3);
    assert_ne!(c["snapshot_id"], a["snapshot_id"]);
    let l = h.send("GET", &format!("{p}/snapshots"), Value::Null).await;
    assert_eq!(l.0, StatusCode::OK);
    assert_eq!(l.2["items"].as_array().unwrap().len(), 2);
    assert_eq!(h.send("GET", &p, Value::Null).await.2["content_version"], 3);
}

// A restore that overwrites without saving current makes reverse restore lose data.
#[tokio::test]
async fn t25_3_http_restore_preserves_current_and_can_reverse() {
    let h = Harness::new().await;
    let p = h.create("original").await;
    let first = h.flush(&p).await;
    let changed_document = json!({"blocks":[{"id":"changed","type":"paragraph","content":[{"type":"text","text":"恢复前的正文"}]}]});
    assert_eq!(
        h.send(
            "PATCH",
            &p,
            json!({"expected_version":1,"title":"before restore","document":changed_document})
        )
        .await
        .0,
        StatusCode::OK
    );
    let r = h.restore(&p, &first, 2).await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.1["etag"], "\"3\"");
    assert_eq!(r.2["title"], "original");
    assert_eq!(r.2["document"], json!({"blocks":[]}));
    let l = h
        .send("GET", &format!("{p}/snapshots"), Value::Null)
        .await
        .2;
    let saved = l["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["content_version"] == 2)
        .unwrap();
    assert_eq!(saved["title"], "before restore");
    assert_eq!(saved["document"], changed_document);
    assert_eq!(saved["reason"], "before_restore");
    let r = h.restore(&p, saved, 3).await;
    assert_eq!(r.0, StatusCode::OK);
    assert_eq!(r.2["title"], "before restore");
    assert_eq!(r.2["document"], changed_document);
    assert_eq!(r.2["content_version"], 4);
    assert_eq!(h.flush(&p).await["content_version"], 4);
}

// Stale CAS, cross-note IDs and write failures must not preserve phantom history.
#[tokio::test]
async fn t25_http_rejected_restores_have_no_effects_and_database_failure_rolls_back() {
    let h = Harness::new().await;
    let p = h.create("one").await;
    let q = h.create("other").await;
    let first = h.flush(&p).await;
    let foreign = h.flush(&q).await;
    h.update(&p, 1, "unsnapped").await;
    let before = h.state().await;
    let r = h.restore(&p, &first, 1).await;
    assert_eq!(r.0, StatusCode::CONFLICT);
    assert_eq!(r.2["current_version"], 2);
    assert_eq!(h.restore(&p, &foreign, 2).await.0, StatusCode::NOT_FOUND);
    assert_eq!(h.state().await, before);
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT t25_reject_event CHECK (topic <> 'note.changed') NOT VALID").execute(&h.db.pool).await.unwrap();
    assert_eq!(
        h.restore(&p, &first, 2).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
}

// Ignoring scope, sorting ties or bounds causes duplicate/leaked pages or unbounded reads.
#[tokio::test]
async fn t25_http_cursor_is_scoped_validated_and_limits_are_50_100() {
    let h = Harness::new().await;
    let p = h.create("0").await;
    let q = h.create("q").await;
    h.flush(&p).await;
    for v in 1..103 {
        h.update(&p, v, &v.to_string()).await;
        h.flush(&p).await;
    }
    // Force a real sorting tie: a timestamp-only cursor would lose page two.
    sqlx::query("UPDATE note_snapshots SET created_at='2026-01-01T00:00:00Z'")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let l = h
        .send("GET", &format!("{p}/snapshots"), Value::Null)
        .await
        .2;
    assert_eq!(l["items"].as_array().unwrap().len(), 50);
    let cursor = l["next_cursor"].as_str().unwrap();
    assert_eq!(
        h.send(
            "GET",
            &format!("{q}/snapshots?cursor={cursor}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let next = h
        .send(
            "GET",
            &format!("{p}/snapshots?cursor={cursor}&limit=100"),
            Value::Null,
        )
        .await
        .2;
    assert_eq!(next["items"].as_array().unwrap().len(), 53);
    assert!(next["next_cursor"].is_null());
    let mut ids = l["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(next["items"].as_array().unwrap())
        .map(|s| s["snapshot_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 103);
    let cursor_json: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cursor).unwrap()).unwrap();
    for (key, value) in [
        ("v", json!(2)),
        ("created_at", json!(17)),
        ("created_at", json!("bad timestamp")),
        ("snapshot_id", Value::Null),
        ("snapshot_id", json!("bad uuid")),
        ("extra", json!(true)),
    ] {
        let mut invalid = cursor_json.clone();
        invalid[key] = value;
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invalid).unwrap());
        assert_eq!(
            h.send(
                "GET",
                &format!("{p}/snapshots?cursor={encoded}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for query in [
        "limit=0",
        "limit=101",
        "cursor=invalid",
        "cursor=e30",
        "unknown=1",
    ] {
        assert_eq!(
            h.send("GET", &format!("{p}/snapshots?{query}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

// New actions must retain session+CSRF protection and reject invalid body/paths.
#[tokio::test]
async fn t25_http_auth_csrf_validation_and_deleted_or_missing_notes() {
    let h = Harness::new().await;
    let p = h.create("auth").await;
    let s = h.flush(&p).await;
    let restore = format!(
        "{p}/snapshots/{}/restore",
        s["snapshot_id"].as_str().unwrap()
    );
    for (m, p) in [
        ("GET", format!("{p}/snapshots")),
        ("POST", format!("{p}/snapshots")),
        ("POST", restore.clone()),
    ] {
        let r = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(m)
                    .uri(&p)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        if m == "POST" {
            let r = h
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(m)
                        .uri(&p)
                        .header("cookie", &h.cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::FORBIDDEN);
        }
    }
    for b in [
        json!({}),
        json!({"expected_version":null}),
        json!({"expected_version":1,"extra":true}),
    ] {
        assert_eq!(h.send("POST", &restore, b).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        h.send("POST", &restore, json!({"expected_version":0}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        h.send("GET", "/api/v1/notes/invalid/snapshots", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let missing = format!("/api/v1/notes/{}/snapshots", uuid::Uuid::new_v4());
    for m in ["GET", "POST"] {
        assert_eq!(
            h.send(m, &missing, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        h.send("DELETE", &format!("{p}?expected_version=1"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("POST", &restore, json!({"expected_version":2}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    for m in ["GET", "POST"] {
        assert_eq!(
            h.send(m, &format!("{p}/snapshots"), Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
}
