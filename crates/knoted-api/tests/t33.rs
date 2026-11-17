//! Real HTTP and PostgreSQL checks for the note persistence boundary.
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
use uuid::Uuid;

struct Harness {
    db: TestDb,
    app: Router,
    cookie: String,
    csrf: String,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let ctx = AppContext::new(db.pool.clone()).with_clock(std::sync::Arc::new(
            FixedClock::new("2026-11-17T00:00:00Z".parse().unwrap()),
        ));
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
    async fn project(&self) -> Value {
        let (s, h, b) = self
            .send(
                "POST",
                "/api/v1/projects",
                json!({"title":"项目","description":"original"}),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        assert_eq!(h["etag"], "\"1\"");
        assert_eq!(b["version"], 1);
        assert!(b.get("archive_batch_id").is_none());
        b
    }
    async fn task(
        &self,
        project: Option<&Value>,
        status: &str,
        archived: bool,
        deleted: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let mut tx = self.db.pool.begin().await.unwrap();
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'task')")
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tasks(id,project_id,status,archived_at,deleted_at,created_at,updated_at) VALUES($1,$2,$3,CASE WHEN $4 THEN '2026-11-16Z'::timestamptz END, CASE WHEN $5 THEN '2026-11-16Z'::timestamptz END,'2026-11-16Z','2026-11-16Z')").bind(id).bind(project.map(uid)).bind(status).bind(archived).bind(deleted).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        id
    }
    async fn snapshot(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('projects',(SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY id),'[]') FROM projects p),'tasks',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]') FROM tasks t),'events',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY id),'[]') FROM outbox o))").fetch_one(&self.db.pool).await.unwrap()
    }
    async fn action(&self, p: &Value, op: &str, v: i64) -> (StatusCode, HeaderMap, Value) {
        self.send(
            "POST",
            &format!("/api/v1/projects/{}/{op}", uid(p)),
            json!({"expected_version":v}),
        )
        .await
    }
}
fn uid(p: &Value) -> Uuid {
    p["id"].as_str().unwrap().parse().unwrap()
}

// Detects missing atomic terminal-state checks, including individually archived tasks.
#[tokio::test]
async fn t33_1_unfinished_blocks_archive_atomically() {
    let h = Harness::new().await;
    let p = h.project().await;
    let a = h.task(Some(&p), "todo", true, false).await;
    let b = h.task(Some(&p), "in_progress", false, false).await;
    let before = h.snapshot().await;
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::CONFLICT);
    assert_eq!(h.snapshot().await, before);
    sqlx::query("UPDATE tasks SET status='done' WHERE id=$1")
        .bind(a)
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::CONFLICT);
    sqlx::query("UPDATE tasks SET status='cancelled' WHERE id=$1")
        .bind(b)
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (s, etag, p) = h.action(&p, "archive", 1).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(etag["etag"], "\"2\"");
    assert_eq!(p["archived_at"], "2026-11-17T00:00:00Z");
    let state = h.snapshot().await;
    let arow = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == a.to_string())
        .unwrap();
    let brow = state["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == b.to_string())
        .unwrap();
    assert_eq!(arow["version"], 1);
    assert!(arow["archive_batch_id"].is_null());
    assert_eq!(brow["version"], 2);
    assert_eq!(
        brow["archive_batch_id"],
        state["projects"][0]["archive_batch_id"]
    );
    assert_eq!(state["events"].as_array().unwrap().len(), 3);
}

// Detects note/registry/tag cascades and deleting with live terminal/archived tasks.
#[tokio::test]
async fn t33_2_delete_never_deletes_independent_note() {
    let h = Harness::new().await;
    let p = h.project().await;
    let path = format!("/api/v1/projects/{}", uid(&p));
    let (s, _, note) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"note","document":{"blocks":[]}}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s,_,link)=h.send("POST","/api/v1/links",json!({"source_type":"note","source_id":note["id"],"target_type":"project","target_id":p["id"]})).await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let (_, _, tag) = h.send("POST", "/api/v1/tags", json!({"name":"keep"})).await;
    let tagpath = format!("/api/v1/entities/project/{}/tags/{}", uid(&p), uid(&tag));
    assert_eq!(
        h.send("PUT", &tagpath, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    let task = h.task(Some(&p), "done", true, false).await;
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=1"), Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE tasks SET deleted_at=now() WHERE id=$1")
        .bind(task)
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=1"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.send("GET", "/api/v1/projects", Value::Null).await.2["items"],
        json!([])
    );
    assert_eq!(
        h.send("GET", &format!("/api/v1/notes/{}", uid(&note)), Value::Null)
            .await
            .2,
        note
    );
    let target: Uuid = sqlx::query_scalar("SELECT target_id FROM entity_links WHERE id=$1")
        .bind(uid(&link))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(target, uid(&p));
    assert_eq!(h.action(&p, "restore", 2).await.2["version"], 3);
    let (s, _, tags) = h
        .send(
            "GET",
            &format!("/api/v1/entities/project/{}/tags", uid(&p)),
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(tags["items"][0]["id"], tag["id"]);
    let deleted: bool = sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM tasks WHERE id=$1")
        .bind(task)
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert!(deleted);
}

// Detects timestamp-based unarchive and touching deleted/foreign/previously archived tasks.
#[tokio::test]
async fn t33_3_batch_isolation() {
    let h = Harness::new().await;
    let p = h.project().await;
    let other = h.project().await;
    let a = h.task(Some(&p), "done", true, false).await;
    let b = h.task(Some(&p), "done", false, false).await;
    let c = h.task(Some(&p), "done", false, true).await;
    let d = h.task(Some(&other), "todo", false, false).await;
    h.task(None, "todo", false, false).await;
    // A shares the archive clock instant but belongs to a different batch.
    sqlx::query("UPDATE tasks SET archived_at='2026-11-17Z',archive_batch_id=$2 WHERE id=$1")
        .bind(a)
        .bind(Uuid::new_v4())
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.snapshot().await;
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::OK);
    let archived = h.snapshot().await;
    assert_eq!(h.action(&p, "unarchive", 2).await.0, StatusCode::OK);
    let after = h.snapshot().await;
    for id in [a, c, d] {
        let find = |v: &Value| {
            v["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id.to_string())
                .unwrap()
                .clone()
        };
        assert_eq!(find(&before), find(&after));
    }
    let br = after["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == b.to_string())
        .unwrap();
    assert_eq!(br["version"], 3);
    assert!(br["archived_at"].is_null());
    assert!(br["archive_batch_id"].is_null());
    assert_eq!(h.action(&p, "archive", 3).await.0, StatusCode::OK);
    let again = h.snapshot().await;
    let batch = |v: &Value| {
        v["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == p["id"])
            .unwrap()["archive_batch_id"]
            .clone()
    };
    assert_ne!(batch(&archived), batch(&again));
}

// Detects omission/null conflation, lost updates, incorrect no-op versioning and DTO leaks.
#[tokio::test]
async fn t33_crud_cas_and_patch_presence() {
    let h = Harness::new().await;
    let p = h.project().await;
    let path = format!("/api/v1/projects/{}", uid(&p));
    let kind: String = sqlx::query_scalar("SELECT entity_type FROM entity_registry WHERE id=$1")
        .bind(uid(&p))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(kind, "project");
    let (s, _, p) = h
        .send("PATCH", &path, json!({"expected_version":1,"title":"new"}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p["description"], "original");
    assert_eq!(p["version"], 2);
    assert_eq!(
        h.send(
            "PATCH",
            &path,
            json!({"expected_version":2,"description":"changed"})
        )
        .await
        .2["version"],
        3
    );
    let (s, _, p) = h
        .send(
            "PATCH",
            &path,
            json!({"expected_version":3,"description":null}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p["version"], 4);
    assert!(p["description"].is_null());
    let snapshot = h.snapshot().await;
    assert_eq!(
        h.send(
            "PATCH",
            &path,
            json!({"expected_version":4,"description":null})
        )
        .await
        .2["version"],
        4
    );
    assert_eq!(h.snapshot().await, snapshot);
    for (body, status) in [
        (json!({"expected_version":4,"title":null}), 400),
        (json!({"expected_version":4,"title":""}), 422),
        (json!({"expected_version":4}), 422),
        (json!({"expected_version":0,"title":"x"}), 422),
        (json!({"expected_version":4,"extra":1}), 400),
        (json!({"expected_version":4,"title":"\u{0}"}), 422),
    ] {
        assert_eq!(
            h.send("PATCH", &path, body.clone()).await.0.as_u16(),
            status,
            "{body}"
        );
    }
    let (a, b) = tokio::join!(
        h.send("PATCH", &path, json!({"expected_version":4,"title":"a"})),
        h.send("PATCH", &path, json!({"expected_version":4,"title":"b"}))
    );
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let conflict = if a.0 == StatusCode::CONFLICT {
        a.2
    } else {
        b.2
    };
    assert_eq!(conflict["current_version"], 5);
    assert_eq!(h.send("GET", &path, Value::Null).await.1["etag"], "\"5\"");
}

// Detects wrong default archive filtering, unstable tie pagination and unscoped cursors.
#[tokio::test]
async fn t33_list_cursor_scope() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let r = h.project().await;
    let deleted = h.project().await;
    h.action(&p, "archive", 1).await;
    assert_eq!(
        h.send(
            "DELETE",
            &format!("/api/v1/projects/{}?expected_version=1", uid(&deleted)),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let mut expected = vec![
        uid(&p).to_string(),
        uid(&q).to_string(),
        uid(&r).to_string(),
    ];
    expected.sort();
    let (s, _, first) = h.send("GET", "/api/v1/projects?limit=1", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let mut page = first.clone();
    let mut seen = Vec::new();
    loop {
        seen.push(page["items"][0]["id"].as_str().unwrap().to_owned());
        if page["next_cursor"].is_null() {
            break;
        }
        page = h
            .send(
                "GET",
                &format!(
                    "/api/v1/projects?limit=1&cursor={}",
                    page["next_cursor"].as_str().unwrap()
                ),
                Value::Null,
            )
            .await
            .2;
    }
    assert_eq!(seen, expected);
    assert_eq!(
        h.send("GET", "/api/v1/projects?archived=true", Value::Null)
            .await
            .2["items"][0]["id"],
        p["id"]
    );
    assert_eq!(
        h.send("GET", "/api/v1/projects?archived=false", Value::Null)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/projects?archived=false&cursor={cursor}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut parsed: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cursor).unwrap()).unwrap();
    parsed["v"] = json!(2);
    for suffix in [
        "limit=0".to_owned(),
        "limit=101".to_owned(),
        "archived=unknown".to_owned(),
        "cursor=%%%".to_owned(),
        format!("cursor={}", "a".repeat(1025)),
        format!(
            "cursor={}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&parsed).unwrap())
        ),
        format!("cursor={}", URL_SAFE_NO_PAD.encode(b"{}")),
    ] {
        assert_eq!(
            h.send("GET", &format!("/api/v1/projects?{suffix}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{suffix}"
        );
    }
}

// Detects bypasses of archived read-only state in project and T32 operations.
#[tokio::test]
async fn t33_archived_readonly_and_t32_activation() {
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let (_, _, tag) = h.send("POST", "/api/v1/tags", json!({"name":"tag"})).await;
    let linkbody = json!({"source_type":"project","source_id":p["id"],"target_type":"project","target_id":q["id"]});
    let (_, _, link) = h.send("POST", "/api/v1/links", linkbody.clone()).await;
    let t = h.task(Some(&p), "done", false, false).await;
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::OK);
    let path = format!("/api/v1/projects/{}", uid(&p));
    for path in [
        &path,
        &format!("/api/v1/entities/project/{}/tags", uid(&p)),
        &format!("/api/v1/entities/task/{t}/tags"),
        &format!("/api/v1/links?source_type=project&source_id={}", uid(&p)),
    ] {
        assert_eq!(h.send("GET", path, Value::Null).await.0, StatusCode::OK);
    }
    let before = h.snapshot().await;
    assert_eq!(
        h.send("PATCH", &path, json!({"expected_version":2,"title":"bad"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=2"), Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send(
            "PUT",
            &format!("/api/v1/entities/project/{}/tags/{}", uid(&p), uid(&tag)),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send("POST", "/api/v1/links", linkbody).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send(
            "DELETE",
            &format!("/api/v1/links/{}?expected_version=1", uid(&link)),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.snapshot().await, before);
    let (_, _, n) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"n","document":{"blocks":[]}}),
        )
        .await;
    let target = json!({"source_type":"note","source_id":n["id"],"target_type":"project","target_id":p["id"]});
    assert_eq!(
        h.send("POST", "/api/v1/links", target.clone()).await.0,
        StatusCode::CREATED
    );
    assert_eq!(h.action(&p, "unarchive", 2).await.0, StatusCode::OK);
    sqlx::query("UPDATE tasks SET deleted_at=now() WHERE id=$1")
        .bind(t)
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=3"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("POST", "/api/v1/links", target).await.0,
        StatusCode::NOT_FOUND
    );
}

// Detects wrong lifecycle CAS/state transitions, event emission on rejected writes, lock leakage.
#[tokio::test]
async fn t33_restore_and_state_cas() {
    let h = Harness::new().await;
    let p = h.project().await;
    let path = format!("/api/v1/projects/{}", uid(&p));
    for op in ["restore", "unarchive"] {
        assert_eq!(h.action(&p, op, 1).await.0, StatusCode::CONFLICT);
    }
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::OK);
    assert_eq!(h.action(&p, "archive", 2).await.0, StatusCode::CONFLICT);
    assert_eq!(h.action(&p, "unarchive", 2).await.0, StatusCode::OK);
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=3"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let before = h.snapshot().await;
    let (s, _, b) = h.action(&p, "restore", 1).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(b["current_version"], 4);
    assert_eq!(
        h.send("GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.action(&p, "archive", 4).await.0, StatusCode::NOT_FOUND);
    assert_eq!(h.snapshot().await, before);
    let mut tx = h.db.pool.begin().await.unwrap();
    let got: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(33001,'projects'::regclass::oid::integer)",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(got);
    tx.rollback().await.unwrap();
    assert_eq!(h.action(&p, "restore", 4).await.2["version"], 5);
}

// Detects non-atomic outbox insertion for create, lifecycle, and cascading task changes.
#[tokio::test]
async fn t33_transactional_outbox() {
    let h = Harness::new().await;
    let p = h.project().await;
    let t = h.task(Some(&p), "done", false, false).await;
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT t33_reject_task CHECK(topic <> 'task.changed')")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.snapshot().await;
    let (s, _, b) = h.action(&p, "archive", 1).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert!(!b.to_string().contains("t33_reject_task"));
    assert_eq!(h.snapshot().await, before);
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT t33_reject_task")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.action(&p, "archive", 1).await.0, StatusCode::OK);
    assert_eq!(h.action(&p, "unarchive", 2).await.0, StatusCode::OK);
    let events: Vec<(String, Value)> =
        sqlx::query_as("SELECT topic,payload FROM outbox ORDER BY created_at,id")
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(events.len(), 5);
    assert!(events.iter().any(|(topic, b)| topic == "project.changed"
        && b == &json!({"source_id":uid(&p),"content_version":1,"operation":"created"})));
    assert!(events.iter().any(|(topic, b)| topic == "task.changed"
        && b == &json!({"source_id":t,"content_version":3,"operation":"unarchived"})));
    sqlx::query("UPDATE tasks SET deleted_at=now() WHERE id=$1")
        .bind(t)
        .execute(&h.db.pool)
        .await
        .unwrap();
    let path = format!("/api/v1/projects/{}", uid(&p));
    assert_eq!(
        h.send("DELETE", &format!("{path}?expected_version=3"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(h.action(&p, "restore", 4).await.2["version"], 5);
    assert_eq!(
        h.send(
            "PATCH",
            &path,
            json!({"expected_version":5,"description":"private"})
        )
        .await
        .2["version"],
        6
    );
    let events: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE topic='project.changed'")
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(events.len(), 6);
    for (v, op) in [
        (1, "created"),
        (2, "archived"),
        (3, "unarchived"),
        (4, "deleted"),
        (5, "restored"),
        (6, "updated"),
    ] {
        assert!(
            events.contains(&json!({"source_id":uid(&p),"content_version":v,"operation":op})),
            "{op} event must share the write transaction"
        );
    }
    sqlx::query("ALTER TABLE outbox ADD CONSTRAINT t33_reject_project CHECK(topic <> 'project.changed') NOT VALID").execute(&h.db.pool).await.unwrap();
    let registry: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_registry")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send("POST", "/api/v1/projects", json!({"title":"rollback"}))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM entity_registry")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        registry
    );
    let before = h.snapshot().await;
    assert_eq!(
        h.send(
            "PATCH",
            &format!("/api/v1/projects/{}", uid(&p)),
            json!({"expected_version":6,"title":"rollback"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.snapshot().await, before);
}

// Detects missing auth/CSRF, malformed-input mapping and database detail leaks.
#[tokio::test]
async fn t33_auth_csrf_errors() {
    let h = Harness::new().await;
    for method in ["GET", "POST"] {
        let req = Request::builder()
            .method(method)
            .uri("/api/v1/projects")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"x"}"#))
            .unwrap();
        assert_eq!(
            h.app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for origin in [None, Some("https://wrong.invalid")] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/v1/projects")
            .header("cookie", &h.cookie)
            .header("content-type", "application/json");
        if let Some(o) = origin {
            req = req.header("origin", o).header("x-csrf-token", &h.csrf);
        }
        assert_eq!(
            h.app
                .clone()
                .oneshot(req.body(Body::from(r#"{"title":"x"}"#)).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/projects")
        .header("cookie", &h.cookie)
        .header("origin", AuthConfig::default().origin)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"x"}"#))
        .unwrap();
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for (path, body, status) in [
        ("/api/v1/projects", json!({"title":null}), 400),
        ("/api/v1/projects", json!({"title":""}), 422),
        ("/api/v1/projects", json!({"title":"字".repeat(501)}), 422),
        (
            "/api/v1/projects",
            json!({"title":"x","description":"\u{0}"}),
            422,
        ),
        ("/api/v1/projects", json!({"title":"x","unknown":true}), 400),
    ] {
        let (s, _, b) = h.send("POST", path, body).await;
        assert_eq!(s.as_u16(), status);
        assert!(b["request_id"].as_str().is_some());
    }
    assert_eq!(
        h.send("GET", "/api/v1/projects/not-uuid", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/projects/{}", Uuid::new_v4()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let title = format!(" {} ", "字".repeat(498));
    let (s, _, created) = h
        .send("POST", "/api/v1/projects", json!({"title":title}))
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(created["title"], title);
    assert!(created["description"].is_null());
    sqlx::query("ALTER TABLE projects RENAME TO unavailable_projects")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (s, _, b) = h.send("GET", "/api/v1/projects", Value::Null).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!b.to_string().contains("unavailable_projects"));
}

async fn blocked_backend(
    c: &mut sqlx::PgConnection,
    blockers: &[i32],
    excluded: Option<i32>,
) -> Option<i32> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            // Refresh the pg_stat_activity snapshot inside the gate transaction.
            sqlx::query("SELECT pg_stat_clear_snapshot()").execute(&mut *c).await.unwrap();
            let pid: Option<i32> = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND pid IS DISTINCT FROM $2 AND pg_blocking_pids(pid) && $1::integer[] ORDER BY pid LIMIT 1")
                .bind(blockers).bind(excluded).fetch_optional(&mut *c).await.unwrap();
            if let Some(pid) = pid { return pid; }
            tokio::task::yield_now().await;
        }
    }).await.ok()
}

// Detects project-first row locks: with task UUID < project UUID the archive
// would hold project and await task while T32 holds task and awaits project.
#[tokio::test]
async fn t33_bulk_locks_match_t32_uuid_order() {
    use sqlx::Connection;
    let h = Harness::new().await;
    let p = h.project().await;
    let task = Uuid::from_u128(1);
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'task')")
        .bind(task)
        .execute(&h.db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tasks(id,project_id,status,created_at,updated_at) VALUES($1,$2,'done',now(),now())").bind(task).bind(uid(&p)).execute(&h.db.pool).await.unwrap();
    assert!(task < uid(&p));
    let mut gate = sqlx::PgConnection::connect(&knoted_core::db::testing::test_database_url())
        .await
        .unwrap();
    sqlx::query(&format!("SET search_path TO {}, public", h.db.schema()))
        .execute(&mut gate)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    sqlx::query("SELECT id FROM tasks WHERE id=$1 FOR UPDATE")
        .bind(task)
        .execute(&mut *tx)
        .await
        .unwrap();
    let request = |path: String, body: Value| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("cookie", &h.cookie)
            .header("origin", AuthConfig::default().origin)
            .header("x-csrf-token", &h.csrf)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };
    let link = tokio::spawn(h.app.clone().oneshot(request(
        "/api/v1/links".into(),
        json!({"source_type":"project","source_id":p["id"],"target_type":"task","target_id":task}),
    )));
    let link_pid = blocked_backend(&mut tx, &[pid], None).await;
    assert!(link_pid.is_some(), "link must await smallest task row");
    let archive = tokio::spawn(h.app.clone().oneshot(request(
        format!("/api/v1/projects/{}/archive", uid(&p)),
        json!({"expected_version":1}),
    )));
    let arch_pid = blocked_backend(&mut tx, &[pid, link_pid.unwrap()], link_pid).await;
    // While archive is queued, a third connection can still acquire project:
    // this distinguishes global UUID order from an unobserved lucky join.
    let unlocked: Result<bool, sqlx::Error> = sqlx::query_scalar(
        "SELECT count(*)=1 FROM (SELECT id FROM projects WHERE id=$1 FOR UPDATE NOWAIT) p",
    )
    .bind(uid(&p))
    .fetch_one(&mut *tx)
    .await;
    tx.commit().await.unwrap();
    let (link, archive) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (
            link.await.unwrap().unwrap(),
            archive.await.unwrap().unwrap(),
        )
    })
    .await
    .expect("no project/task deadlock");
    assert!(
        arch_pid.is_some(),
        "archive must overlap T32 at the controlled task row"
    );
    assert_eq!(link.status(), StatusCode::CREATED);
    assert_eq!(archive.status(), StatusCode::OK);
    assert!(unlocked.expect("archive must not hold the larger project before the smaller task"));
}
