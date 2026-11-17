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

// Missing final-state guard must reject every live nonterminal child, preserve
// every write on conflict, and count archived children but ignore deleted ones.
#[tokio::test]
async fn t35_3_parent_done_requires_all_live_children_terminal() {
    let h = Harness::new().await;
    let a = h.task(json!({"title":"parent"})).await;
    let c = h.task(json!({"title":"todo","parent_id":a["id"]})).await;
    let d = h
        .task(json!({"title":"progress","parent_id":a["id"],"status":"in_progress"}))
        .await;
    for child in [&c, &d] {
        let before = h.snapshot().await;
        let (s, _, b) = h
            .patch(&a, json!({"expected_version":1,"status":"done"}))
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{b}");
        assert_eq!(b["current_version"], 1);
        assert_eq!(h.snapshot().await, before);
        assert_eq!(
            h.patch(child, json!({"expected_version":1,"status":"done"}))
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        h.patch(&a, json!({"expected_version":1,"status":"done"}))
            .await
            .0,
        StatusCode::OK
    );
    let a = h.task(json!({"title":"deleted case"})).await;
    let c = h.task(json!({"title":"deleted","parent_id":a["id"]})).await;
    assert_eq!(h.delete(&c, 1).await, StatusCode::NO_CONTENT);
    let c = h
        .task(json!({"title":"archived","parent_id":a["id"]}))
        .await;
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.snapshot().await;
    assert_eq!(
        h.patch(&a, json!({"expected_version":1,"status":"done"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.snapshot().await, before);
    sqlx::query("UPDATE tasks SET status='cancelled' WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.patch(&a, json!({"expected_version":1,"status":"done"}))
            .await
            .0,
        StatusCode::OK
    );
    let a = h.task(json!({"title":"empty"})).await;
    assert_eq!(
        h.patch(&a, json!({"expected_version":1,"status":"done"}))
            .await
            .0,
        StatusCode::OK
    );
}

// Detects checking status before merging parent/title/date or omitting create guards.
#[tokio::test]
async fn t35_done_parent_child_create_reopen_reparent() {
    let h = Harness::new().await;
    let project = h.project().await;
    let p = h
        .task(json!({"title":"done parent","status":"done","project_id":project["id"]}))
        .await;
    let before = h.snapshot().await;
    assert_eq!(
        h.send(
            "POST",
            "/api/v1/tasks",
            json!({"title":"default todo","parent_id":p["id"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.snapshot().await, before);
    for status in ["todo", "in_progress"] {
        let before = h.snapshot().await;
        let (s, _, b) = h
            .send(
                "POST",
                "/api/v1/tasks",
                json!({"title":"rejected child","parent_id":p["id"],"status":status}),
            )
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{b}");
        assert!(b.get("current_version").is_none());
        assert_eq!(h.snapshot().await, before);
    }
    for status in ["done", "cancelled"] {
        let c = h
            .task(json!({"title":"terminal","parent_id":p["id"],"status":status}))
            .await;
        for reopen in ["todo", "in_progress"] {
            let before = h.snapshot().await;
            let (s, _, b) = h
                .patch(&c, json!({"expected_version":1,"status":reopen}))
                .await;
            assert_eq!(s, StatusCode::CONFLICT, "{b}");
            assert_eq!(b["current_version"], 1);
            assert_eq!(h.snapshot().await, before);
        }
    }
    let c = h.task(json!({"title":"independent"})).await;
    let before = h.snapshot().await;
    assert_eq!(h.patch(&c,json!({"expected_version":1,"title":"mixed","parent_id":p["id"],"due":date("2026-11-17",json!("10:30"))})).await.0,StatusCode::CONFLICT);
    assert_eq!(h.snapshot().await, before);
    h.clock.advance(chrono::Duration::minutes(1));
    let (s,_,c)=h.patch(&c,json!({"expected_version":1,"title":"mixed","status":"cancelled","parent_id":p["id"],"due":date("2026-11-17",json!("10:30"))})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["version"], 2);
    assert_eq!(c["title"], "mixed");
    assert_eq!(c["parent_id"], p["id"]);
    assert_eq!(c["project_id"], project["id"]);
    assert_eq!(c["updated_at"], "2026-11-17T00:01:00Z");
    let after = h.snapshot().await;
    assert_eq!(
        after["events"].as_array().unwrap().len(),
        before["events"].as_array().unwrap().len() + 1
    );
    assert_eq!(
        h.patch(&p, json!({"expected_version":1,"status":"todo"}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        h.patch(&c, json!({"expected_version":2,"status":"in_progress"}))
            .await
            .0,
        StatusCode::OK
    );
}

// Historical invalid states must permit explicit repair; same-value PATCH must
// still evaluate the final set, and cancelled parents have no done-only rule.
#[tokio::test]
async fn t35_mixed_update_atomic_and_repair() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"parent"})).await;
    let c = h.task(json!({"title":"child","parent_id":p["id"]})).await;
    sqlx::query("UPDATE tasks SET status='done' WHERE id=$1")
        .bind(uid(&p))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.snapshot().await;
    assert_eq!(
        h.patch(&c, json!({"expected_version":1,"status":"todo"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.patch(&p, json!({"expected_version":1,"status":"done"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.snapshot().await, before);
    assert_eq!(
        h.patch(
            &c,
            json!({"expected_version":1,"status":"done","title":"repair"})
        )
        .await
        .0,
        StatusCode::OK
    );
    sqlx::query("UPDATE tasks SET status='todo' WHERE id=$1")
        .bind(uid(&c))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.patch(&p, json!({"expected_version":1,"status":"cancelled"}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        h.patch(&c, json!({"expected_version":2,"status":"in_progress"}))
            .await
            .0,
        StatusCode::OK
    );
    // A single PATCH detaches the nonterminal child and repairs the old parent.
    sqlx::query("UPDATE tasks SET status='done' WHERE id=$1")
        .bind(uid(&p))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.patch(&c, json!({"expected_version":3,"parent_id":null}))
            .await
            .0,
        StatusCode::OK
    );
}

// Fixed-offset or wall-time-only validation incorrectly accepts these IANA gap/fold values.
#[tokio::test]
async fn t35_real_iana_invalid_local_time_http() {
    let h = Harness::new().await;
    for (d, time) in [("1991-04-14", "02:30"), ("1991-09-15", "01:30")] {
        for field in ["start", "due"] {
            let before = h.snapshot().await;
            let mut body = json!({"title":"invalid local time"});
            body[field] = date(d, json!(time));
            let (status, _, error) = h.send("POST", "/api/v1/tasks", body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(error["code"], "validation_failed");
            assert_eq!(error["field_errors"][0]["field"], "start/due");
            assert_eq!(h.snapshot().await, before);
        }
    }
}

async fn completion_race(operation: &str, child_first: bool) {
    use sqlx::Connection;
    let h = Harness::new().await;
    let p = h.task(json!({"title":"parent"})).await;
    let c = match operation {
        "create" => None,
        "reopen" => Some(
            h.task(json!({"title":"child","parent_id":p["id"],"status":"done"}))
                .await,
        ),
        "reparent" => Some(h.task(json!({"title":"independent"})).await),
        _ => unreachable!(),
    };
    // Gate the parent row so the first real writer holds relation
    // admission while the second is provably waiting on that writer.
    let min = uid(&p);
    let mut gate = gate_connection(&h).await;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    sqlx::query("SELECT id FROM tasks WHERE id=$1 FOR UPDATE")
        .bind(min)
        .execute(&mut *tx)
        .await
        .unwrap();
    let complete = request(
        &h,
        "PATCH",
        &format!("/api/v1/tasks/{}", uid(&p)),
        json!({"expected_version":1,"status":"done"}),
    );
    let child = match c.as_ref() {
        None => request(
            &h,
            "POST",
            "/api/v1/tasks",
            json!({"title":"new child","parent_id":p["id"]}),
        ),
        Some(c) => request(
            &h,
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(c)),
            if operation == "reopen" {
                json!({"expected_version":1,"status":"todo"})
            } else {
                json!({"expected_version":1,"parent_id":p["id"]})
            },
        ),
    };
    let (first_req, second_req) = if child_first {
        (child, complete)
    } else {
        (complete, child)
    };
    let first = tokio::spawn(h.app.clone().oneshot(first_req));
    let writer = blocked(&mut tx, pid, None).await;
    let second = tokio::spawn(h.app.clone().oneshot(second_req));
    blocked(&mut tx, writer, Some(writer)).await;
    tx.commit().await.unwrap();
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (
            first.await.unwrap().unwrap(),
            second.await.unwrap().unwrap(),
        )
    })
    .await
    .expect("no deadlock");
    assert_eq!(
        first.status(),
        if child_first && operation == "create" {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        }
    );
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "{operation},child_first={child_first}"
    );
    let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks p JOIN tasks c ON c.parent_id=p.id WHERE p.deleted_at IS NULL AND p.status='done' AND c.deleted_at IS NULL AND c.status NOT IN ('done','cancelled'))").fetch_one(&h.db.pool).await.unwrap();
    assert!(!invalid);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='task.changed'")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(events, if operation == "create" { 2 } else { 3 });
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
#[tokio::test]
async fn t35_completion_vs_create() {
    for first in [true, false] {
        completion_race("create", first).await;
    }
}
#[tokio::test]
async fn t35_completion_vs_reopen() {
    for first in [true, false] {
        completion_race("reopen", first).await;
    }
}
#[tokio::test]
async fn t35_completion_vs_reparent() {
    for first in [true, false] {
        completion_race("reparent", first).await;
    }
}

// Detects locking only the moved child and parents: old/new siblings must join
// discovery before any globally larger row is locked.
#[tokio::test]
async fn t35_reparent_locks_old_and_new_siblings_in_global_order() {
    use sqlx::Connection;
    let h = Harness::new().await;
    let ids: Vec<Uuid> = (1..=6).map(Uuid::from_u128).collect();
    for id in &ids {
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'task')")
            .bind(id)
            .execute(&h.db.pool)
            .await
            .unwrap();
    }
    // old sibling=1, new sibling=2, old parent=3, new parent=4, target=5;
    // An additional new-parent sibling participates in the discovered set.
    for id in &ids[2..4] {
        sqlx::query(
            "INSERT INTO tasks(id,title,created_at,updated_at) VALUES($1,'parent',now(),now())",
        )
        .bind(id)
        .execute(&h.db.pool)
        .await
        .unwrap();
    }
    for (id, parent) in [
        (ids[0], ids[2]),
        (ids[1], ids[3]),
        (ids[4], ids[2]),
        (ids[5], ids[3]),
    ] {
        sqlx::query("INSERT INTO tasks(id,parent_id,title,created_at,updated_at) VALUES($1,$2,'child',now(),now())").bind(id).bind(parent).execute(&h.db.pool).await.unwrap();
    }
    for sibling in [ids[0], ids[1]] {
        let mut gate = gate_connection(&h).await;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut gate)
            .await
            .unwrap();
        let mut tx = gate.begin().await.unwrap();
        sqlx::query("SELECT id FROM tasks WHERE id=$1 FOR UPDATE")
            .bind(sibling)
            .execute(&mut *tx)
            .await
            .unwrap();
        let moving = tokio::spawn(h.app.clone().oneshot(request(
            &h,
            "PATCH",
            &format!("/api/v1/tasks/{}", ids[4]),
            json!({"expected_version":1,"parent_id":ids[3]}),
        )));
        blocked(&mut tx, pid, None).await;
        let unlocked = sqlx::query(
            "SELECT id FROM tasks WHERE id IN ($1,$2,$3) ORDER BY id FOR UPDATE NOWAIT",
        )
        .bind(ids[2])
        .bind(ids[3])
        .bind(ids[4])
        .fetch_all(&mut *tx)
        .await;
        let larger_free = unlocked.as_ref().is_ok_and(|rows| rows.len() == 3);
        if larger_free {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(10), moving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(larger_free, "siblings must precede parents and target");
        assert_eq!(response.status(), StatusCode::OK);
        // Reset only this explicit test fixture for the second old/new sibling gate.
        sqlx::query("UPDATE tasks SET parent_id=$2,version=1 WHERE id=$1")
            .bind(ids[4])
            .bind(ids[2])
            .execute(&h.db.pool)
            .await
            .unwrap();
    }
}

// Integrates the pure helper with a real persisted Task and AppContext's clock;
// preserving the supplied calendar object must not persist a derived overdue flag.
#[tokio::test]
async fn t35_date_persistence_and_context_clock() {
    use knoted_core::tasks::{dates::is_overdue, service};
    let h = Harness::new().await;
    let due = date("2026-11-17", Value::Null);
    let created = h.task(json!({"title":"calendar deadline","due":due})).await;
    assert_eq!(created["due"], due);
    assert!(created.get("overdue").is_none());
    let stored: Value = sqlx::query_scalar("SELECT to_jsonb(t) FROM tasks t WHERE id=$1")
        .bind(uid(&created))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(stored["due"], due);
    assert!(stored.get("overdue").is_none());
    let ctx = AppContext::new(h.db.pool.clone()).with_clock(std::sync::Arc::new(h.clock.clone()));
    let task = service::get(&ctx, service::GetTask { id: uid(&created) })
        .await
        .unwrap();
    h.clock.set("2026-11-17T15:59:59.999999Z".parse().unwrap());
    assert!(!is_overdue(&task, ctx.now()).unwrap());
    h.clock.advance(chrono::Duration::microseconds(1));
    assert!(is_overdue(&task, ctx.now()).unwrap());
}
