//! Asset transport acceptance uses the real router and PostgreSQL.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router;
use knoted_core::{context::AppContext, db::testing::TestDb};
use tower::ServiceExt;

#[tokio::test]
async fn t26_1_anonymous_download_and_upload_are_protected() {
    let db = TestDb::create().await.unwrap();
    let app = build_router(AppContext::new(db.pool.clone()));
    for (method, path) in [
        (
            "GET",
            "/api/v1/assets/550e8400-e29b-41d4-a716-446655440000/content",
        ),
        ("POST", "/api/v1/assets"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["request_id"], request_id);
    }
}

use axum::{Router, http::HeaderMap};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Duration, Utc};
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::FixedClock,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

struct Harness {
    db: TestDb,
    app: Router,
    ctx: AppContext,
    root: tempfile::TempDir,
    cookie: String,
    csrf: String,
    clock: Arc<FixedClock>,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(FixedClock::new(Utc::now()));
        // Every test owns private filesystem storage.
        let ctx = AppContext::new(db.pool.clone())
            .with_clock(clock.clone())
            .with_asset_root(root.path());
        let mut tx = db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            app: build_router(ctx.clone()),
            ctx,
            root,
            cookie: format!(
                "knoted_session={}; knoted_csrf={}",
                session.token, session.csrf_token
            ),
            csrf: session.csrf_token,
            db,
            clock,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        bytes: Vec<u8>,
        key: Option<&str>,
        csrf: bool,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", &self.cookie)
            .header("content-type", content_type);
        if csrf {
            builder = builder
                .header("origin", AuthConfig::default().origin)
                .header("x-csrf-token", &self.csrf);
        }
        if let Some(key) = key {
            builder = builder.header("idempotency-key", key);
        }
        let r = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(bytes)).unwrap())
            .await
            .unwrap();
        let (s, h) = (r.status(), r.headers().clone());
        (
            s,
            h,
            to_bytes(r.into_body(), 24 * 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
    }
    async fn upload(&self, bytes: &[u8], key: &str, hash: Option<&str>) -> (StatusCode, Value) {
        let body = multipart(bytes, hash);
        let (s, h, b) = self
            .call(
                "POST",
                "/api/v1/assets",
                "multipart/form-data; boundary=t26",
                body,
                Some(key),
                true,
            )
            .await;
        let value: Value = serde_json::from_slice(&b).unwrap();
        if !s.is_success() {
            assert_eq!(value["request_id"], h["x-request-id"].to_str().unwrap());
        }
        (s, value)
    }
    async fn json(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let (s, _, b) = self
            .call(
                method,
                path,
                "application/json",
                serde_json::to_vec(&body).unwrap(),
                None,
                true,
            )
            .await;
        (
            s,
            if b.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&b).unwrap()
            },
        )
    }
    async fn state(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('assets',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM assets a),'notes',(SELECT jsonb_agg(to_jsonb(n) ORDER BY id) FROM notes n),'snapshots',(SELECT jsonb_agg(to_jsonb(s) ORDER BY snapshot_id) FROM note_snapshots s),'refs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY asset_id,ref_kind,ref_id) FROM asset_refs r),'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM outbox o))").fetch_one(&self.db.pool).await.unwrap()
    }
}
fn png() -> Vec<u8> {
    STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC").unwrap()
}
fn multipart(bytes: &[u8], hash: Option<&str>) -> Vec<u8> {
    let mut b=b"--t26\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../../attack.html\"\r\nContent-Type: text/html\r\n\r\n".to_vec();
    b.extend_from_slice(bytes);
    if let Some(hash) = hash {
        b.extend_from_slice(
            format!("\r\n--t26\r\nContent-Disposition: form-data; name=\"sha256\"\r\n\r\n{hash}")
                .as_bytes(),
        );
    }
    b.extend_from_slice(b"\r\n--t26--\r\n");
    b
}
fn doc(asset: &str) -> Value {
    json!({"blocks":[{"id":"first","type":"image","asset_id":asset},{"id":"second","type":"image","asset_id":asset}]})
}

#[tokio::test]
async fn t26_upload_read_metadata_paths_and_idempotency() {
    let h = Harness::new().await;
    let bytes = png();
    let (a, b) = tokio::join!(
        h.upload(&bytes, "same", None),
        h.upload(&bytes, "same", None)
    );
    assert_eq!(a.0, StatusCode::CREATED, "{}", a.1);
    assert_eq!(a, b);
    let id = Uuid::parse_str(a.1["id"].as_str().unwrap()).unwrap();
    assert_eq!(a.1["mime"], "image/png");
    assert_eq!(a.1["size"], bytes.len());
    assert_eq!(a.1["state"], "ready");
    assert!(a.1.get("storage_key").is_none());
    assert!(a.1.get("asset_id").is_none());
    assert_eq!(h.upload(&bytes, "same", None).await, a);
    // Presence of the optional digest is part of the request, even when correct.
    let sha = a.1["sha256"].as_str().unwrap();
    assert_eq!(
        h.upload(&bytes, "same", Some(sha)).await.0,
        StatusCode::CONFLICT
    );
    let path = format!("/api/v1/assets/{id}/content");
    let r = h
        .call(
            "GET",
            &path,
            "application/octet-stream",
            vec![],
            None,
            false,
        )
        .await;
    assert_eq!(r.0, StatusCode::OK);
    assert_eq!(r.2, bytes);
    assert_eq!(r.1["content-type"], "image/png");
    assert_eq!(r.1["x-content-type-options"], "nosniff");
    assert!(
        r.1["content-disposition"]
            .to_str()
            .unwrap()
            .contains(&id.to_string())
    );
    assert!(r.1.get("set-cookie").is_none());
    assert!(r.1.contains_key("etag"));
    let entries = std::fs::read_dir(h.root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(entries.contains(&id.to_string()));
    assert!(!h.root.path().join("attack.html").exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM assets")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn t26_http_validation_csrf_and_error_envelopes() {
    let h = Harness::new().await;
    for (bytes, hash, status) in [
        (
            b"not an image".to_vec(),
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (png()[..40].to_vec(), None, StatusCode::UNPROCESSABLE_ENTITY),
        (
            png(),
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            vec![0; 20 * 1024 * 1024 + 1],
            None,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        assert_eq!(
            h.upload(&bytes, &Uuid::new_v4().to_string(), hash).await.0,
            status
        );
    }
    for (key, ctype, body, csrf, status) in [
        (
            Some("key"),
            "multipart/form-data; boundary=t26",
            multipart(&png(), None),
            false,
            StatusCode::FORBIDDEN,
        ),
        (
            None,
            "multipart/form-data; boundary=t26",
            multipart(&png(), None),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(""),
            "multipart/form-data; boundary=t26",
            multipart(&png(), None),
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            Some("key"),
            "multipart/form-data",
            vec![],
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            Some("key"),
            "multipart/form-data; boundary=t26",
            b"broken".to_vec(),
            true,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (s, h, b) = h
            .call("POST", "/api/v1/assets", ctype, body, key, csrf)
            .await;
        assert_eq!(s, status);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["request_id"], h["x-request-id"].to_str().unwrap());
    }
    for (path, status) in [
        (
            "/api/v1/assets/no-uuid/content".to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/v1/assets/{}/content", Uuid::new_v4()),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (s, h, b) = h.call("GET", &path, "", vec![], None, false).await;
        assert_eq!(s, status);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["request_id"], h["x-request-id"].to_str().unwrap());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM assets")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn t26_3_history_current_trash_restore_prune_and_invalid_write_rollback() {
    let h = Harness::new().await;
    let asset = h.upload(&png(), "history", None).await.1;
    let id = asset["id"].as_str().unwrap();
    let (s, n) = h
        .json(
            "POST",
            "/api/v1/notes",
            json!({"title":"images","document":doc(id)}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    let p = format!("/api/v1/notes/{}", n["id"].as_str().unwrap());
    let refs =
        || sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_refs").fetch_one(&h.db.pool);
    assert_eq!(refs().await.unwrap(), 1);
    let snapshot = h
        .json("POST", &format!("{p}/snapshots"), Value::Null)
        .await
        .1;
    assert_eq!(refs().await.unwrap(), 2);
    assert_eq!(
        h.json(
            "PATCH",
            &p,
            json!({"expected_version":1,"document":{"blocks":[]}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        refs().await.unwrap(),
        1,
        "historical reference survives current removal"
    );
    let before = h.state().await;
    assert_eq!(
        h.json(
            "PATCH",
            &p,
            json!({"expected_version":2,"document":doc(&Uuid::new_v4().to_string())})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(h.state().await, before);
    assert_eq!(
        h.json(
            "POST",
            "/api/v1/notes",
            json!({"title":"bad","document":doc(&Uuid::new_v4().to_string())})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(h.state().await, before);
    let restored = h
        .json(
            "POST",
            &format!(
                "{p}/snapshots/{}/restore",
                snapshot["snapshot_id"].as_str().unwrap()
            ),
            json!({"expected_version":2}),
        )
        .await;
    assert_eq!(restored.0, StatusCode::OK, "{}", restored.1);
    assert_eq!(restored.1["document"], doc(id));
    assert_eq!(refs().await.unwrap(), 2);
    let deleted = h
        .json("DELETE", &format!("{p}?expected_version=3"), Value::Null)
        .await;
    assert_eq!(deleted.0, StatusCode::NO_CONTENT);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_refs WHERE ref_kind='trash'")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        h.json(
            "POST",
            &format!("{p}/restore"),
            json!({"expected_version":4})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_refs WHERE ref_kind='current'")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    h.clock.advance(Duration::days(91));
    knoted_core::notes::history::maintain_history(
        &h.ctx,
        knoted_core::notes::history::MaintainHistory { recover: true },
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM asset_refs WHERE ref_kind='snapshot'")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1,
        "old snapshot refs pruned while latest persists"
    );
}

#[tokio::test]
async fn t26_conflict_precedes_new_payload_validation_and_keys_are_opaque() {
    let h = Harness::new().await;
    let first = h.upload(&png(), "opaque key with spaces", None).await;
    assert_eq!(first.0, StatusCode::CREATED, "{}", first.1);
    assert_eq!(
        h.upload(
            &png(),
            "opaque key with spaces",
            Some("0000000000000000000000000000000000000000000000000000000000000000")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.upload(b"different invalid bytes", "opaque key with spaces", None)
            .await
            .0,
        StatusCode::CONFLICT
    );
}

// Any late note event/snapshot/ref failure must roll back versions and refs,
// including zero_ref_at used by the future retention policy.
#[tokio::test]
async fn t26_forced_note_transaction_failure_preserves_document_history_and_refs() {
    let h = Harness::new().await;
    let (s, asset) = h.upload(&png(), "rollback", None).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = asset["id"].as_str().unwrap();
    let (_, note) = h
        .json(
            "POST",
            "/api/v1/notes",
            json!({"title":"kept","document":doc(id)}),
        )
        .await;
    let p = format!("/api/v1/notes/{}", note["id"].as_str().unwrap());
    let snapshot = h
        .json("POST", &format!("{p}/snapshots"), Value::Null)
        .await
        .1;
    let before = h.state().await;
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT t26_fail_event CHECK (topic <> 'note.changed') NOT VALID").execute(&h.db.pool).await.unwrap();
    assert_eq!(
        h.json(
            "PATCH",
            &p,
            json!({"expected_version":1,"document":{"blocks":[]}})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
    assert_eq!(
        h.json("DELETE", &format!("{p}?expected_version=1"), Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
    assert_eq!(
        h.json(
            "POST",
            &format!(
                "{p}/snapshots/{}/restore",
                snapshot["snapshot_id"].as_str().unwrap()
            ),
            json!({"expected_version":1})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT t26_fail_event")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.json(
            "PATCH",
            &p,
            json!({"expected_version":1,"document":{"blocks":[]}})
        )
        .await
        .0,
        StatusCode::OK
    );
    // Reference insertion fails after the snapshot row was inserted: both roll back.
    let before = h.state().await;
    sqlx::query("ALTER TABLE asset_refs ADD CONSTRAINT t26_fail_ref CHECK (ref_kind <> 'current') NOT VALID").execute(&h.db.pool).await.unwrap();
    assert_eq!(
        h.json(
            "POST",
            &format!(
                "{p}/snapshots/{}/restore",
                snapshot["snapshot_id"].as_str().unwrap()
            ),
            json!({"expected_version":2})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
}

#[tokio::test]
async fn t26_multipart_duplicate_fields_unknown_fields_and_hash_shape_rejected() {
    let h = Harness::new().await;
    let first = multipart(&png(), None);
    let mut duplicate = first[..first.len() - 9].to_vec();
    duplicate.extend_from_slice(&multipart(&png(), None));
    let unknown =
        b"--t26\r\nContent-Disposition: form-data; name=\"unknown\"\r\n\r\ndata\r\n--t26--\r\n"
            .to_vec();
    for body in [duplicate, unknown, multipart(&png(), Some("not-sha256"))] {
        let (s, heads, b) = h
            .call(
                "POST",
                "/api/v1/assets",
                "multipart/form-data; boundary=t26",
                body,
                Some("bad"),
                true,
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["request_id"], heads["x-request-id"].to_str().unwrap());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM assets")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn t68_disconnected_http_upload_finishes_registration_before_backup() {
    let h = Harness::new().await;
    let mut barrier = h.db.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE assets IN SHARE MODE")
        .execute(&mut *barrier)
        .await
        .unwrap();
    let bytes = png();
    let mut client = Box::pin(h.upload(&bytes, "disconnect", None));
    tokio::select! {
        response = &mut client => panic!("registration passed the SQL barrier: {}", response.0),
        ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if std::fs::read_dir(h.root.path()).unwrap().flatten().any(|entry| entry.file_type().unwrap().is_file()) { break; }
                tokio::task::yield_now().await;
            }
        }) => ready.unwrap(),
    }
    drop(client);
    let refused = knoted_core::maintenance::exclusive(
        h.ctx.admission_pool(),
        std::time::Duration::from_millis(100),
    )
    .await
    .is_err();
    barrier.rollback().await.unwrap();
    assert!(
        refused,
        "the HTTP owner must still protect the asset registration"
    );
    let guard = knoted_core::maintenance::exclusive(
        h.ctx.admission_pool(),
        std::time::Duration::from_secs(3),
    )
    .await
    .unwrap();
    let asset: Uuid = sqlx::query_scalar("SELECT id FROM assets WHERE state='ready'")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(h.root.path().join(asset.to_string())).unwrap(),
        bytes
    );
    drop(guard);
    h.db.drop().await;
}
