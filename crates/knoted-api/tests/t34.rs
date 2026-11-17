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
    clock: FixedClock,
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
            clock,
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
    async fn task(&self, b: Value) -> Value {
        let (s, h, b) = self.send("POST", "/api/v1/tasks", b).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        assert_eq!(h["etag"], "\"1\"");
        b
    }
    async fn patch(&self, t: &Value, b: Value) -> (StatusCode, HeaderMap, Value) {
        self.send("PATCH", &format!("/api/v1/tasks/{}", uid(t)), b)
            .await
    }
    async fn delete(&self, t: &Value, v: i64) -> StatusCode {
        self.send(
            "DELETE",
            &format!("/api/v1/tasks/{}?expected_version={v}", uid(t)),
            Value::Null,
        )
        .await
        .0
    }
    async fn snapshot(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('tasks',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]') FROM tasks t),'registry',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id),'[]') FROM entity_registry r),'events',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY id),'[]') FROM outbox o))").fetch_one(&self.db.pool).await.unwrap()
    }
}
fn uid(t: &Value) -> Uuid {
    t["id"].as_str().unwrap().parse().unwrap()
}
fn date(d: &str, time: Value) -> Value {
    json!({"date":d,"time":time,"timezone":"Asia/Shanghai"})
}

// Detects absent independent CRUD, wrong DTO defaults/CAS/events and physical deletion.
#[tokio::test]
async fn t34_3_independent_crud_and_contract() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"独立"})).await;
    for k in [
        "project_id",
        "parent_id",
        "description",
        "start",
        "due",
        "archived_at",
        "deleted_at",
    ] {
        assert!(t.get(k).unwrap().is_null(), "{k}");
    }
    assert_eq!(t["status"], "todo");
    assert_eq!(t["priority"], "none");
    assert_eq!(t.as_object().unwrap().len(), 14);
    assert_eq!(t["created_at"], "2026-11-17T00:00:00Z");
    assert_eq!(t["updated_at"], t["created_at"]);
    let path = format!("/api/v1/tasks/{}", uid(&t));
    assert_eq!(h.send("GET", &path, Value::Null).await.2, t);
    let kind: String = sqlx::query_scalar("SELECT entity_type FROM entity_registry WHERE id=$1")
        .bind(uid(&t))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(kind, "task");
    h.clock.advance(chrono::Duration::minutes(3));
    let (a, b) = tokio::join!(
        h.patch(&t, json!({"expected_version":1,"title":"a"})),
        h.patch(&t, json!({"expected_version":1,"title":"b"}))
    );
    let mut codes = [a.0.as_u16(), b.0.as_u16()];
    codes.sort();
    assert_eq!(codes, [200, 409]);
    let winner = if a.0 == StatusCode::OK {
        a.2.clone()
    } else {
        b.2.clone()
    };
    let loser = if a.0 == StatusCode::CONFLICT {
        a.2
    } else {
        b.2
    };
    assert_eq!(loser["current_version"], 2);
    assert_eq!(winner["updated_at"], "2026-11-17T00:03:00Z");
    let before = h.snapshot().await;
    assert_eq!(
        h.patch(&t, json!({"expected_version":2,"title":winner["title"]}))
            .await
            .1["etag"],
        "\"2\""
    );
    assert_eq!(h.snapshot().await, before);
    assert_eq!(h.delete(&t, 1).await, StatusCode::CONFLICT);
    assert_eq!(h.delete(&t, 2).await, StatusCode::NO_CONTENT);
    assert_eq!(
        h.send("GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let ev: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE topic='task.changed' ORDER BY payload->>'operation'",
    )
    .fetch_all(&h.db.pool)
    .await
    .unwrap();
    assert_eq!(ev.len(), 3);
    for (v, op) in [(1, "created"), (2, "updated"), (3, "deleted")] {
        assert!(ev.contains(&json!({"source_id":uid(&t),"content_version":v,"operation":op})));
    }
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM entity_registry WHERE id=$1)")
            .bind(uid(&t))
            .fetch_one(&h.db.pool)
            .await
            .unwrap()
    );
}
// Detects grandchildren/self/reparent cycles and project inheritance bypasses.
#[tokio::test]
async fn t34_1_single_level_and_reparent() {
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let a = h.task(json!({"title":"a","project_id":p["id"]})).await;
    let b = h.task(json!({"title":"b","project_id":q["id"]})).await;
    let c = h.task(json!({"title":"c","parent_id":a["id"]})).await;
    assert_eq!(c["project_id"], p["id"]);
    assert_eq!(
        h.send(
            "POST",
            "/api/v1/tasks",
            json!({"title":"grandchild","parent_id":c["id"]})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for (t, parent) in [
        (&a, a["id"].clone()),
        (&a, b["id"].clone()),
        (&c, c["id"].clone()),
    ] {
        assert_eq!(
            h.patch(t, json!({"expected_version":1,"parent_id":parent}))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (s, _, c) = h
        .patch(&c, json!({"expected_version":1,"parent_id":b["id"]}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["project_id"], q["id"]);
    let (s, _, c) = h
        .patch(&c, json!({"expected_version":2,"parent_id":null}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(c["parent_id"].is_null());
    assert_eq!(c["project_id"], q["id"]);
    assert_eq!(
        h.send(
            "POST",
            "/api/v1/tasks",
            json!({"title":"missing","parent_id":Uuid::new_v4()})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&a))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send(
            "POST",
            "/api/v1/tasks",
            json!({"title":"archived","parent_id":a["id"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
// Detects omission/null collapse, cross-project child placement and direct child moves.
#[tokio::test]
async fn t34_2_project_inheritance_and_patch_presence() {
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let a = h.task(json!({"title":"a","project_id":p["id"]})).await;
    for project in [Value::Null, q["id"].clone()] {
        assert_eq!(
            h.send(
                "POST",
                "/api/v1/tasks",
                json!({"title":"bad","parent_id":a["id"],"project_id":project})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let c=h.task(json!({"title":"c","parent_id":a["id"],"description":"text","start":date("2026-11-16",Value::Null),"due":date("2026-11-17",json!("12:00"))})).await;
    for project in [Value::Null, q["id"].clone()] {
        assert_eq!(
            h.patch(&c, json!({"expected_version":1,"project_id":project}))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (s, _, c) = h
        .patch(
            &c,
            json!({"expected_version":1,"description":null,"start":null}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(c["description"].is_null());
    assert!(c["start"].is_null());
    assert_eq!(c["due"], date("2026-11-17", json!("12:00")));
    let (s, _, c) = h
        .patch(
            &c,
            json!({"expected_version":2,"parent_id":null,"project_id":null,"due":null}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(c["project_id"].is_null());
    assert!(c["parent_id"].is_null());
    assert!(c["due"].is_null());
    let root = h.task(json!({"title":"root"})).await;
    let child = h
        .task(json!({"title":"child","parent_id":root["id"],"project_id":null}))
        .await;
    assert!(child["project_id"].is_null());
}
// Detects partial project migration and mutation of archived/deleted child lifecycle/content.
#[tokio::test]
async fn t34_parent_project_move_atomic_children() {
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let a = h.task(json!({"title":"a","project_id":p["id"]})).await;
    let c = h.task(json!({"title":"c","parent_id":a["id"]})).await;
    let d = h
        .task(json!({"title":"d","parent_id":a["id"],"description":"retained"}))
        .await;
    let (status, _, tag) = h
        .send("POST", "/api/v1/tags", json!({"name":"child tag"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        h.send(
            "PUT",
            &format!("/api/v1/entities/task/{}/tags/{}", uid(&d), uid(&tag)),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, _, note) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"linked child","document":{"blocks":[]}}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = h.send("POST", "/api/v1/links", json!({"source_type":"note","source_id":note["id"],"target_type":"task","target_id":d["id"]})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(h.delete(&d, 1).await, StatusCode::NO_CONTENT);
    sqlx::query("UPDATE tasks SET archived_at='2026-11-16Z',archive_batch_id=$2 WHERE id=$1")
        .bind(uid(&d))
        .bind(Uuid::from_u128(99))
        .execute(&h.db.pool)
        .await
        .unwrap();
    // Fail a child's event after the parent event/write: the entire migration
    // and every identity/content row must roll back together.
    sqlx::query(&format!("ALTER TABLE outbox ADD CONSTRAINT t34_reject_child CHECK (NOT (topic='task.changed' AND payload->>'source_id'='{}' AND payload->>'operation'='updated')) NOT VALID", uid(&c))).execute(&h.db.pool).await.unwrap();
    let before = h.snapshot().await;
    assert_eq!(
        h.patch(&a, json!({"expected_version":1,"project_id":q["id"]}))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.snapshot().await, before);
    sqlx::query("ALTER TABLE outbox DROP CONSTRAINT t34_reject_child")
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.clock.advance(chrono::Duration::minutes(1));
    let (s, _, a) = h
        .patch(&a, json!({"expected_version":1,"project_id":q["id"],"title":"renamed","priority":"medium","start":date("2026-11-17",Value::Null)}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(a["version"], 2);
    let rows: Vec<(Uuid, Option<Uuid>, i64)> =
        sqlx::query_as("SELECT id,project_id,version FROM tasks ORDER BY id")
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert!(rows.contains(&(uid(&a), Some(uid(&q)), 2)));
    assert!(rows.contains(&(uid(&c), Some(uid(&q)), 2)));
    assert!(rows.contains(&(uid(&d), Some(uid(&q)), 3)));
    let updated: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT updated_at FROM tasks")
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert!(updated.iter().all(|t| {
        *t == "2026-11-17T00:01:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    }));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM task_tags WHERE entity_id=$1")
            .bind(uid(&d))
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM entity_links WHERE target_id=$1")
            .bind(uid(&d))
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );

    let deleted: Value = sqlx::query_scalar("SELECT to_jsonb(t) FROM tasks t WHERE id=$1")
        .bind(uid(&d))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert!(!deleted["deleted_at"].is_null());
    assert!(!deleted["archived_at"].is_null());
    assert_eq!(deleted["description"], "retained");
    assert_eq!(deleted["archive_batch_id"], Uuid::from_u128(99).to_string());
    assert_eq!(
        h.patch(&c, json!({"expected_version":1,"title":"stale"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.patch(&a, json!({"expected_version":2,"project_id":null}))
            .await
            .0,
        StatusCode::OK
    );
    let projects: Vec<i64> = sqlx::query_scalar("SELECT version FROM projects")
        .fetch_all(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(projects, vec![1, 1]);
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.snapshot().await;
    assert_eq!(
        h.patch(&a, json!({"expected_version":3,"project_id":p["id"]}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.snapshot().await, before);
    let child_events: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE topic='task.changed' AND payload->>'source_id'=$1",
    )
    .bind(uid(&d).to_string())
    .fetch_all(&h.db.pool)
    .await
    .unwrap();
    for (version, operation) in [
        (1, "created"),
        (2, "deleted"),
        (3, "updated"),
        (4, "updated"),
    ] {
        assert!(child_events.contains(
            &json!({"source_id":uid(&d),"content_version":version,"operation":operation})
        ));
    }
    let ev: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='task.changed'")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(ev, 10);
}
// Detects ignoring archived/done children when deleting their parent, or deleting links/tags.
#[tokio::test]
async fn t34_parent_delete_with_surviving_child() {
    let h = Harness::new().await;
    let a = h.task(json!({"title":"a"})).await;
    let c = h
        .task(json!({"title":"c","parent_id":a["id"],"status":"done"}))
        .await;
    assert_eq!(h.delete(&a, 1).await, StatusCode::CONFLICT);
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.delete(&a, 1).await, StatusCode::CONFLICT);
    sqlx::query("UPDATE tasks SET archived_at=NULL WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.delete(&c, 1).await, StatusCode::NO_CONTENT);
    let (_, _, n) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"note","document":{"blocks":[]}}),
        )
        .await;
    let (_, _, tag) = h.send("POST", "/api/v1/tags", json!({"name":"tag"})).await;
    assert_eq!(
        h.send(
            "PUT",
            &format!("/api/v1/entities/task/{}/tags/{}", uid(&a), uid(&tag)),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let(s,_,_)=h.send("POST","/api/v1/links",json!({"source_type":"note","source_id":n["id"],"target_type":"task","target_id":a["id"]})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(h.delete(&a, 1).await, StatusCode::NO_CONTENT);
    assert_eq!(
        h.send("GET", &format!("/api/v1/notes/{}", uid(&n)), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM task_tags")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM entity_links")
            .fetch_one(&h.db.pool)
            .await
            .unwrap(),
        1
    );
}
// Detects loose enums/dates and checking only the PATCH half of a date interval.
#[tokio::test]
async fn t34_input_enum_date_and_merge_validation() {
    let h = Harness::new().await;
    for (field, value, status) in [
        ("status", json!("unknown"), 400),
        ("status", Value::Null, 400),
        ("priority", json!("urgent"), 400),
        ("priority", Value::Null, 400),
        ("title", json!(""), 422),
        ("title", json!("字".repeat(501)), 422),
        ("title", json!("a\u{0}"), 422),
        ("description", json!("a\u{0}"), 422),
        ("unknown", json!(true), 400),
    ] {
        let mut b = json!({"title":"x"});
        b[field] = value;
        assert_eq!(
            h.send("POST", "/api/v1/tasks", b.clone()).await.0.as_u16(),
            status,
            "{b}"
        );
    }
    for v in [
        json!({"date":"2026-01-01","timezone":"Asia/Shanghai"}),
        json!({"time":null,"timezone":"Asia/Shanghai"}),
        json!({"date":"2026-01-01","time":null}),
        json!({"date":"2026-01-01","time":null,"timezone":"Asia/Shanghai","extra":1}),
    ] {
        assert_eq!(
            h.send("POST", "/api/v1/tasks", json!({"title":"x","due":v}))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    for v in [
        date("2026-02-30", Value::Null),
        date("2026-11-17", json!("24:00")),
        date("2026-1-1", Value::Null),
        json!({"date":"2026-11-17","time":null,"timezone":"UTC"}),
    ] {
        assert_eq!(
            h.send("POST", "/api/v1/tasks", json!({"title":"x","start":v}))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let t=h.task(json!({"title":"x","start":date("2026-11-17",Value::Null),"due":date("2026-11-17",Value::Null)})).await;
    assert_eq!(t["due"], date("2026-11-17", Value::Null));
    assert_eq!(
        h.patch(
            &t,
            json!({"expected_version":1,"start":date("2026-11-18",json!("00:01"))})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        h.patch(
            &t,
            json!({"expected_version":1,"start":date("2026-11-18",json!("00:00"))})
        )
        .await
        .0,
        StatusCode::OK
    );
    for b in [
        json!({"expected_version":2}),
        json!({"expected_version":0,"title":"a"}),
    ] {
        assert_eq!(h.patch(&t, b).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    for field in ["title", "status", "priority"] {
        let mut b = json!({"expected_version":2});
        b[field] = Value::Null;
        assert_eq!(h.patch(&t, b).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        h.patch(
            &t,
            json!({"expected_version":2,"due":null,"start":date("2027-01-01",Value::Null)})
        )
        .await
        .0,
        StatusCode::OK
    );
}
// Detects default root-only filtering, omitted filters and cursor/filter mismatches.
#[tokio::test]
async fn t34_task_list_all_filters_and_cursor() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let h = Harness::new().await;
    let p = h.project().await;
    let a = h.task(json!({"title":"a","project_id":p["id"]})).await;
    let c = h
        .task(json!({"title":"c","parent_id":a["id"],"status":"in_progress","priority":"high"}))
        .await;
    let b = h
        .task(json!({"title":"b","status":"done","priority":"low"}))
        .await;
    let d = h.task(json!({"title":"deleted"})).await;
    assert_eq!(h.delete(&d, 1).await, StatusCode::NO_CONTENT);
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&b))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (s, _, first) = h.send("GET", "/api/v1/tasks?limit=1", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let mut page = first.clone();
    let mut seen = vec![];
    loop {
        seen.push(uid(&page["items"][0]));
        if page["next_cursor"].is_null() {
            break;
        }
        page = h
            .send(
                "GET",
                &format!(
                    "/api/v1/tasks?limit=1&cursor={}",
                    page["next_cursor"].as_str().unwrap()
                ),
                Value::Null,
            )
            .await
            .2;
    }
    let mut expected = vec![uid(&a), uid(&b), uid(&c)];
    expected.sort();
    assert_eq!(seen, expected);
    for query in [
        "status=in_progress".to_owned(),
        "priority=high".into(),
        format!("parent_id={}", uid(&a)),
        format!(
            "project_id={}&status=in_progress&priority=high&archived=false",
            uid(&p)
        ),
    ] {
        let (s, _, b) = h
            .send("GET", &format!("/api/v1/tasks?{query}"), Value::Null)
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b["items"].as_array().unwrap().len(), 1);
        assert_eq!(b["items"][0]["id"], c["id"]);
    }
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/tasks?project_id={}", uid(&p)),
            Value::Null
        )
        .await
        .2["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        h.send("GET", "/api/v1/tasks?archived=true", Value::Null)
            .await
            .2["items"][0]["id"],
        b["id"]
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    for q in [
        "status=todo".to_owned(),
        "priority=none".into(),
        format!("project_id={}", uid(&p)),
        format!("parent_id={}", uid(&a)),
        "archived=false".into(),
    ] {
        assert_eq!(
            h.send(
                "GET",
                &format!("/api/v1/tasks?{q}&cursor={cursor}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let parsed: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cursor).unwrap()).unwrap();
    for key in ["project_id", "parent_id", "status", "priority", "archived"] {
        let mut bad = parsed.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert_eq!(
            h.send(
                "GET",
                &format!(
                    "/api/v1/tasks?cursor={}",
                    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&bad).unwrap())
                ),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for q in [
        "limit=0",
        "limit=101",
        "parent_id=null",
        "status=invalid",
        "priority=bad",
        "cursor=%%%",
        "archived=x",
    ] {
        assert_eq!(
            h.send("GET", &format!("/api/v1/tasks?{q}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

// Detects missing readonly checks on target/parent/project and non-atomic event failures.
#[tokio::test]
async fn t34_readonly_cas_clock_and_outbox() {
    let h = Harness::new().await;
    let p = h.project().await;
    let a = h.task(json!({"title":"a","project_id":p["id"]})).await;
    let c = h.task(json!({"title":"c","parent_id":a["id"]})).await;
    for (table, id) in [
        ("projects", uid(&p)),
        ("tasks", uid(&a)),
        ("tasks", uid(&c)),
    ] {
        sqlx::query(&format!("UPDATE {table} SET archived_at=now() WHERE id=$1"))
            .bind(id)
            .execute(&h.db.pool)
            .await
            .unwrap();
        let before = h.snapshot().await;
        let (s, _, b) = h
            .patch(&c, json!({"expected_version":1,"title":"denied"}))
            .await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(b["current_version"], 1);
        assert_eq!(h.delete(&c, 1).await, StatusCode::CONFLICT);
        assert_eq!(
            h.send("GET", &format!("/api/v1/tasks/{}", uid(&c)), Value::Null)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(h.snapshot().await, before);
        sqlx::query(&format!("UPDATE {table} SET archived_at=NULL WHERE id=$1"))
            .bind(id)
            .execute(&h.db.pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "ALTER TABLE outbox ADD CONSTRAINT t34_reject CHECK(topic <> 'task.changed') NOT VALID",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let before = h.snapshot().await;
    for (method, path, b) in [
        ("POST", "/api/v1/tasks".into(), json!({"title":"rollback"})),
        (
            "PATCH",
            format!("/api/v1/tasks/{}", uid(&c)),
            json!({"expected_version":1,"title":"rollback"}),
        ),
        (
            "DELETE",
            format!("/api/v1/tasks/{}?expected_version=1", uid(&c)),
            Value::Null,
        ),
    ] {
        let (s, _, b) = h.send(method, &path, b).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!b.to_string().contains("t34_reject"));
        assert!(b["request_id"].is_string());
        assert_eq!(h.snapshot().await, before);
    }
    let mut tx = h.db.pool.begin().await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT pg_try_advisory_xact_lock(33001,'projects'::regclass::oid::integer)"
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap()
    );
    tx.rollback().await.unwrap();
}
// Detects unprotected task routes, malformed-input leaks and persistence error details.
#[tokio::test]
async fn t34_auth_csrf_and_errors() {
    let h = Harness::new().await;
    for method in ["GET", "POST"] {
        let req = Request::builder()
            .method(method)
            .uri("/api/v1/tasks")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"x"}"#))
            .unwrap();
        assert_eq!(
            h.app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for (origin, csrf) in [
        (None, None),
        (Some("https://wrong.invalid"), Some(h.csrf.as_str())),
        (Some(AuthConfig::default().origin.as_str()), None),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/v1/tasks")
            .header("cookie", &h.cookie)
            .header("content-type", "application/json");
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        if let Some(c) = csrf {
            req = req.header("x-csrf-token", c);
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
    for path in ["/api/v1/tasks/not-uuid", "/api/v1/tasks?parent_id=bad"] {
        assert_eq!(
            h.send("GET", path, Value::Null).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/tasks/{}", Uuid::new_v4()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("ALTER TABLE tasks RENAME TO unavailable_tasks")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (s, _, b) = h.send("GET", "/api/v1/tasks", Value::Null).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!b.to_string().contains("unavailable_tasks"));
    assert!(b["request_id"].is_string());
}

// Replays the real ALTER migration over the exact T33 table shape. Unknown
// pre-T34 rows must survive a refused upgrade, without made-up titles.
#[tokio::test]
async fn t34_migration_empty_upgrade_and_unknown_rows_preserved() {
    let h = Harness::new().await;
    let old = include_str!("../../../migrations/0009_t33_projects.sql");
    let old = &old[old.find("CREATE TABLE tasks (").unwrap()..];
    for populated in [false, true] {
        // The replay starts from the T33 shape. Later cards add tables that
        // reference `tasks`, so those are dropped first; a new dependent table
        // must be listed here rather than hidden behind CASCADE.
        sqlx::raw_sql("DROP TABLE IF EXISTS activity_revisions, activities, tasks")
            .execute(&h.db.pool)
            .await
            .unwrap();
        sqlx::raw_sql(old).execute(&h.db.pool).await.unwrap();
        if populated {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'task')")
                .bind(id)
                .execute(&h.db.pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO tasks(id,created_at,updated_at) VALUES($1,'2026-11-16Z','2026-11-16Z')").bind(id).execute(&h.db.pool).await.unwrap();
        }
        let before: Value =
            sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM tasks t")
                .fetch_one(&h.db.pool)
                .await
                .unwrap();
        let mut tx = h.db.pool.begin().await.unwrap();
        let result = sqlx::raw_sql(include_str!("../../../migrations/0010_t34_tasks.sql"))
            .execute(&mut *tx)
            .await;
        if populated {
            assert!(result.is_err());
            tx.rollback().await.unwrap();
            let after: Value =
                sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(t)),'[]') FROM tasks t")
                    .fetch_one(&h.db.pool)
                    .await
                    .unwrap();
            assert_eq!(after, before);
            assert_eq!(after.as_array().unwrap().len(), 1);
            assert!(after[0].get("title").is_none());
        } else {
            result.unwrap();
            tx.commit().await.unwrap();
            let t = h.task(json!({"title":"upgraded"})).await;
            assert_eq!(t["title"], "upgraded");
        }
    }
}

async fn gate_connection(h: &Harness) -> sqlx::PgConnection {
    use sqlx::Connection;
    let mut c = sqlx::PgConnection::connect(&knoted_core::db::testing::test_database_url())
        .await
        .unwrap();
    sqlx::query(&format!("SET search_path TO {}, public", h.db.schema()))
        .execute(&mut c)
        .await
        .unwrap();
    c
}
async fn blocked(c: &mut sqlx::PgConnection, by: i32, exclude: Option<i32>) -> i32 {
    tokio::time::timeout(std::time::Duration::from_secs(10),async{loop{sqlx::query("SELECT pg_stat_clear_snapshot()").execute(&mut *c).await.unwrap();let pid:Option<i32>=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND pid IS DISTINCT FROM $2 AND $1=ANY(pg_blocking_pids(pid)) ORDER BY pid LIMIT 1").bind(by).bind(exclude).fetch_optional(&mut *c).await.unwrap();if let Some(pid)=pid{return pid;}tokio::task::yield_now().await;}}).await.expect("real backend must reach controlled wait")
}
fn request(h: &Harness, method: &str, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", &h.cookie)
        .header("origin", AuthConfig::default().origin)
        .header("x-csrf-token", &h.csrf)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}
async fn parent_race(deleting: bool, create_first: bool) {
    use sqlx::Connection;
    let h = Harness::new().await;
    let p = h.project().await;
    let q = h.project().await;
    let a = h.task(json!({"title":"parent","project_id":p["id"]})).await;
    let mut gate = gate_connection(&h).await;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    let (id, table) = if uid(&a) < uid(&p) {
        (uid(&a), "tasks")
    } else {
        (uid(&p), "projects")
    };
    sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let create_req = request(
        &h,
        "POST",
        "/api/v1/tasks",
        json!({"title":"racing child","parent_id":a["id"]}),
    );
    let change_req = if deleting {
        request(
            &h,
            "DELETE",
            &format!("/api/v1/tasks/{}?expected_version=1", uid(&a)),
            Value::Null,
        )
    } else {
        request(
            &h,
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&a)),
            json!({"expected_version":1,"project_id":q["id"]}),
        )
    };
    let (create, change) = if create_first {
        let create = tokio::spawn(h.app.clone().oneshot(create_req));
        let writer = blocked(&mut tx, pid, None).await;
        let change = tokio::spawn(h.app.clone().oneshot(change_req));
        blocked(&mut tx, writer, Some(writer)).await;
        (create, change)
    } else {
        let change = tokio::spawn(h.app.clone().oneshot(change_req));
        let writer = blocked(&mut tx, pid, None).await;
        let create = tokio::spawn(h.app.clone().oneshot(create_req));
        blocked(&mut tx, writer, Some(writer)).await;
        (create, change)
    };
    tx.commit().await.unwrap();
    let (create, change) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (
            create.await.unwrap().unwrap(),
            change.await.unwrap().unwrap(),
        )
    })
    .await
    .unwrap();
    assert_eq!(
        create.status(),
        if deleting && !create_first {
            StatusCode::CONFLICT
        } else {
            StatusCode::CREATED
        }
    );
    assert_eq!(
        change.status(),
        if deleting && create_first {
            StatusCode::CONFLICT
        } else if deleting {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::OK
        }
    );
    let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks c JOIN tasks p ON p.id=c.parent_id WHERE c.deleted_at IS NULL AND (p.deleted_at IS NOT NULL OR c.project_id IS DISTINCT FROM p.project_id))").fetch_one(&h.db.pool).await.unwrap();
    assert!(!invalid);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='task.changed'")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        count,
        if deleting {
            2
        } else if create_first {
            4
        } else {
            3
        }
    );
}
// Removing relation admission or discovering children before admission breaks these schedules.
#[tokio::test]
async fn t34_parent_move_vs_child_create() {
    parent_race(false, true).await;
    parent_race(false, false).await;
}
#[tokio::test]
async fn t34_parent_delete_vs_child_create() {
    parent_race(true, true).await;
    parent_race(true, false).await;
}

// Detects project-first or per-table locking during cross-project moves: a T32
// link is already queued on the smallest task, and must still acquire both projects.
#[tokio::test]
async fn t34_multitable_uuid_lock_order() {
    use sqlx::Connection;
    let h = Harness::new().await;
    let task = Uuid::from_u128(1);
    let p = Uuid::from_u128(2);
    let q = Uuid::from_u128(3);
    for (id, kind) in [(task, "task"), (p, "project"), (q, "project")] {
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,$2)")
            .bind(id)
            .bind(kind)
            .execute(&h.db.pool)
            .await
            .unwrap();
    }
    for id in [p, q] {
        sqlx::query("INSERT INTO projects(id,title,created_at,updated_at) VALUES($1,'lock project',now(),now())").bind(id).execute(&h.db.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO tasks(id,project_id,title,created_at,updated_at) VALUES($1,$2,'lock task',now(),now())").bind(task).bind(p).execute(&h.db.pool).await.unwrap();
    let mut gate = gate_connection(&h).await;
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
    let link = tokio::spawn(h.app.clone().oneshot(request(
        &h,
        "POST",
        "/api/v1/links",
        json!({"source_type":"project","source_id":q,"target_type":"task","target_id":task}),
    )));
    let link_pid = blocked(&mut tx, pid, None).await;
    let moving = tokio::spawn(h.app.clone().oneshot(request(
        &h,
        "PATCH",
        &format!("/api/v1/tasks/{task}"),
        json!({"expected_version":1,"project_id":q}),
    )));
    let move_pid=tokio::time::timeout(std::time::Duration::from_secs(10),async{loop{sqlx::query("SELECT pg_stat_clear_snapshot()").execute(&mut *tx).await.unwrap();let found:Option<i32>=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND pid<>$1 AND pg_blocking_pids(pid) && $2::int[] ORDER BY pid LIMIT 1").bind(link_pid).bind(vec![pid,link_pid]).fetch_optional(&mut *tx).await.unwrap();if let Some(found)=found{break found;}tokio::task::yield_now().await;}}).await.unwrap();
    assert_ne!(move_pid, link_pid);
    let unlocked =
        sqlx::query("SELECT id FROM projects WHERE id IN ($1,$2) ORDER BY id FOR UPDATE NOWAIT")
            .bind(p)
            .bind(q)
            .fetch_all(&mut *tx)
            .await;
    let projects_free = unlocked
        .as_ref()
        .map(|rows| rows.len() == 2)
        .unwrap_or(false);
    if projects_free {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    let (link, moving) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (link.await.unwrap().unwrap(), moving.await.unwrap().unwrap())
    })
    .await
    .expect("global UUID order must not deadlock");
    assert!(
        projects_free,
        "no larger project may be locked ahead of task"
    );
    assert_eq!(link.status(), StatusCode::CREATED);
    assert_eq!(moving.status(), StatusCode::OK);
}

// Detects accepting absent/deleted project/parent references and leaking a
// referenced entity's version as the new or updated target's CAS version.
#[tokio::test]
async fn t34_reference_states_fail_closed() {
    let h = Harness::new().await;
    let project = h.project().await;
    let parent = h
        .task(json!({"title":"parent","project_id":project["id"]}))
        .await;
    let other = h.task(json!({"title":"other"})).await;
    for field in ["project_id", "parent_id"] {
        let mut b = json!({"title":"missing"});
        b[field] = json!(Uuid::new_v4());
        assert_eq!(
            h.send("POST", "/api/v1/tasks", b).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let mut b = json!({"expected_version":1});
        b[field] = json!(Uuid::new_v4());
        assert_eq!(h.patch(&other, b).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    for (table, id) in [("tasks", uid(&parent)), ("projects", uid(&project))] {
        for field in ["archived_at", "deleted_at"] {
            sqlx::query(&format!(
                "UPDATE {table} SET {field}=now(),version=7 WHERE id=$1"
            ))
            .bind(id)
            .execute(&h.db.pool)
            .await
            .unwrap();
            let before = h.snapshot().await;
            let (s, _, body) = h
                .send(
                    "POST",
                    "/api/v1/tasks",
                    json!({"title":"blocked","parent_id":parent["id"]}),
                )
                .await;
            assert_eq!(s, StatusCode::CONFLICT);
            assert!(body.get("current_version").is_none());
            let (s, _, body) = h
                .patch(
                    &other,
                    json!({"expected_version":1,"parent_id":parent["id"]}),
                )
                .await;
            assert_eq!(s, StatusCode::CONFLICT);
            assert_eq!(body["current_version"], 1);
            assert_eq!(h.snapshot().await, before);
            sqlx::query(&format!("UPDATE {table} SET {field}=NULL WHERE id=$1"))
                .bind(id)
                .execute(&h.db.pool)
                .await
                .unwrap();
        }
    }
    // Historical inconsistent relationship must not be repaired by a write that
    // would otherwise bypass the original parent's/project's readonly state.
    let child = h
        .task(json!({"title":"child","parent_id":parent["id"]}))
        .await;
    sqlx::query("UPDATE tasks SET project_id=NULL WHERE id=$1")
        .bind(uid(&child))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.patch(
            &child,
            json!({"expected_version":1,"parent_id":null,"title":"bypass"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

// A parent cascade must not repair a live child's historical inheritance or
// let that child escape a readonly source project. Each case exercises PATCH
// after an explicit historical SQL fixture and compares the full write snapshot.
async fn mismatched_child_move(source: &str, deleted_child: bool) {
    let h = Harness::new().await;
    let original = h.project().await;
    let alternate = h.project().await;
    let destination = h.project().await;
    let parent = h
        .task(json!({"title":"parent","project_id":original["id"]}))
        .await;
    let child = h
        .task(json!({"title":"child","parent_id":parent["id"],"description":"preserved"}))
        .await;
    if deleted_child {
        assert_eq!(h.delete(&child, 1).await, StatusCode::NO_CONTENT);
    }
    let source_id = match source {
        "null" => None,
        "destination" => Some(uid(&destination)),
        "active" | "archived" | "deleted" => Some(uid(&alternate)),
        _ => unreachable!("test source must be a declared fixture"),
    };
    sqlx::query("UPDATE tasks SET project_id=$2 WHERE id=$1")
        .bind(uid(&child))
        .bind(source_id)
        .execute(&h.db.pool)
        .await
        .unwrap();
    match source {
        "archived" => {
            sqlx::query("UPDATE projects SET archived_at=now() WHERE id=$1")
                .bind(uid(&alternate))
                .execute(&h.db.pool)
                .await
                .unwrap();
        }
        "deleted" => {
            sqlx::query("UPDATE projects SET deleted_at=now() WHERE id=$1")
                .bind(uid(&alternate))
                .execute(&h.db.pool)
                .await
                .unwrap();
        }
        _ => {}
    }
    let before = h.snapshot().await;
    let (status, _, body) = h
        .patch(
            &parent,
            json!({"expected_version":1,"project_id":destination["id"]}),
        )
        .await;
    if !deleted_child {
        assert_eq!(status, StatusCode::CONFLICT, "source={source}: {body}");
        assert_eq!(body["current_version"], 1);
        assert_eq!(
            h.snapshot().await,
            before,
            "rejected cascade must preserve tasks, registry and outbox"
        );
    } else {
        assert_eq!(status, StatusCode::OK, "deleted child exception: {body}");
        assert_eq!(body["version"], 2);
        let after = h.snapshot().await;
        let find_child = |snapshot: &Value| {
            snapshot["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["id"] == child["id"])
                .unwrap()
                .clone()
        };
        let mut expected = find_child(&before);
        expected["project_id"] = destination["id"].clone();
        expected["version"] = json!(3);
        assert_eq!(
            find_child(&after),
            expected,
            "trash child changes only project/version/time"
        );
        assert_eq!(after["registry"], before["registry"]);
        assert_eq!(
            after["events"].as_array().unwrap().len(),
            before["events"].as_array().unwrap().len() + 2
        );
        assert!(after["events"].as_array().unwrap().iter().any(|e| e["topic"] == "task.changed" && e["payload"] == json!({"source_id":uid(&child),"content_version":3,"operation":"updated"})));
    }
}
#[tokio::test]
async fn t34_parent_move_rejects_mismatched_active_child_project() {
    mismatched_child_move("active", false).await;
}
#[tokio::test]
async fn t34_parent_move_rejects_mismatched_archived_child_project() {
    mismatched_child_move("archived", false).await;
}
#[tokio::test]
async fn t34_parent_move_rejects_mismatched_deleted_child_project() {
    mismatched_child_move("deleted", false).await;
}
#[tokio::test]
async fn t34_parent_move_rejects_mismatched_null_child_project() {
    mismatched_child_move("null", false).await;
}
#[tokio::test]
async fn t34_parent_move_rejects_mismatched_child_already_at_destination() {
    mismatched_child_move("destination", false).await;
}
#[tokio::test]
async fn t34_parent_move_preserves_mismatched_deleted_child_exception() {
    mismatched_child_move("archived", true).await;
}
