//! Real HTTP and PostgreSQL checks for task follow-up notes and system events.
//!
//! Scenarios required by the T36 card:
//! - `t36_1`: system events are appended inside the task transaction and are
//!   immutable through the HTTP edit/delete operations.
//! - `t36_2`: a manual note keeps its superseded content reachable through the
//!   revisions endpoint after an edit, and after a delete the record is only a
//!   marker whose content stays in history.
//! - `t36_3`: a rejected edit never leaves an orphan revision behind.
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
    async fn task(&self, b: Value) -> Value {
        let (s, _, b) = self.send("POST", "/api/v1/tasks", b).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        b
    }
    async fn create(&self, task: &Value, body: &str) -> (StatusCode, Value) {
        let (s, _, b) = self
            .send(
                "POST",
                &format!("/api/v1/tasks/{}/activities", uid(task)),
                json!({"body":body}),
            )
            .await;
        (s, b)
    }
    async fn edit(
        &self,
        task: &Value,
        activity: &Value,
        expected: i64,
        body: &str,
    ) -> (StatusCode, Value) {
        self.patch(
            task,
            activity,
            json!({"expected_version":expected,"body":body}),
        )
        .await
    }
    async fn patch(&self, task: &Value, activity: &Value, body: Value) -> (StatusCode, Value) {
        let (s, _, b) = self
            .send(
                "PATCH",
                &format!("/api/v1/tasks/{}/activities/{}", uid(task), uid(activity)),
                body,
            )
            .await;
        (s, b)
    }
    async fn delete_activity(&self, task: &Value, activity: &Value, v: i64) -> StatusCode {
        self.send(
            "DELETE",
            &format!(
                "/api/v1/tasks/{}/activities/{}?expected_version={v}",
                uid(task),
                uid(activity)
            ),
            Value::Null,
        )
        .await
        .0
    }
    async fn list(&self, task: &Value, query: &str) -> (StatusCode, Value) {
        let (s, _, b) = self
            .send(
                "GET",
                &format!("/api/v1/tasks/{}/activities{query}", uid(task)),
                Value::Null,
            )
            .await;
        (s, b)
    }
    async fn revisions(&self, task: &Value, activity: &Value, query: &str) -> (StatusCode, Value) {
        let (s, _, b) = self
            .send(
                "GET",
                &format!(
                    "/api/v1/tasks/{}/activities/{}/revisions{query}",
                    uid(task),
                    uid(activity)
                ),
                Value::Null,
            )
            .await;
        (s, b)
    }
    async fn revision_count(&self, activity: &Value) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM activity_revisions WHERE activity_id=$1")
            .bind(uid(activity))
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
}
fn uid(v: &Value) -> Uuid {
    v["id"].as_str().unwrap().parse().unwrap()
}

// A rejected edit and a rejected delete must never touch a system event, and a
// task update that fails its CAS must not append a second system event.
#[tokio::test]
async fn t36_1_system_events_are_appended_and_immutable() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"跟进对象"})).await;
    let (s, _, _) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&t)),
            json!({"expected_version":1,"status":"in_progress"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    let (s, page) = h.list(&t, "").await;
    assert_eq!(s, StatusCode::OK);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{page}");
    let event = items[0].clone();
    assert_eq!(event["task_id"], t["id"]);
    assert_eq!(event["author_kind"], "system");
    assert_eq!(event["version"], 1);
    assert!(event["deleted_at"].is_null());
    let body = event["body"].as_str().unwrap().to_owned();
    assert!(body.contains("in_progress"), "{body}");

    let (s, err) = h
        .patch(&t, &event, json!({"expected_version":1,"body":"被改写"}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "version_conflict");
    assert!(
        err.get("current_version").is_none(),
        "an immutable event has no competing version: {err}"
    );
    assert_eq!(h.delete_activity(&t, &event, 1).await, StatusCode::CONFLICT);

    let (_, page) = h.list(&t, "").await;
    assert_eq!(page["items"][0]["body"], body.as_str());
    assert_eq!(page["items"][0]["version"], 1);
    assert!(page["items"][0]["deleted_at"].is_null());
    assert_eq!(h.revision_count(&event).await, 0);
    let (s, revs) = h.revisions(&t, &event, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(revs["items"].as_array().unwrap().len(), 0);

    // The CAS failure must roll back the system append with the task update.
    let (s, _, _) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&t)),
            json!({"expected_version":1,"status":"done"}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, page) = h.list(&t, "").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1, "{page}");

    // Another task never sees this task's system event.
    let other = h.task(json!({"title":"另一个"})).await;
    let (_, page) = h.list(&other, "").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 0, "{page}");

    // A terminal transition appends exactly one more immutable event.
    h.clock.advance(chrono::Duration::minutes(1));
    let (s, _, _) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&t)),
            json!({"expected_version":2,"status":"done"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, page) = h.list(&t, "").await;
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{page}");
    let bodies: Vec<_> = items
        .iter()
        .map(|i| {
            (
                i["author_kind"].as_str().unwrap(),
                i["body"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        bodies
            .iter()
            .any(|(k, b)| *k == "system" && b.contains("done")),
        "{page}"
    );
}

// An edit must keep the superseded content in history; a delete must keep only a
// marker in the list while history still carries every former content version.
#[tokio::test]
async fn t36_2_manual_notes_keep_history_across_edit_and_delete() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"跟进对象"})).await;
    let (s, note) = h.create(&t, "第一次跟进").await;
    assert_eq!(s, StatusCode::CREATED, "{note}");
    assert_eq!(note["task_id"], t["id"]);
    assert_eq!(note["author_kind"], "manual");
    assert_eq!(note["body"], "第一次跟进");
    assert_eq!(note["version"], 1);
    assert!(note["deleted_at"].is_null());
    assert_eq!(note["created_at"], note["updated_at"]);
    assert_eq!(note["created_at"], "2026-11-17T00:00:00Z");
    assert_eq!(h.revision_count(&note).await, 0);

    h.clock.advance(chrono::Duration::minutes(5));
    let (s, edited) = h.edit(&t, &note, 1, "第二次跟进").await;
    assert_eq!(s, StatusCode::OK, "{edited}");
    assert_eq!(edited["body"], "第二次跟进");
    assert_eq!(edited["version"], 2);
    assert_eq!(edited["created_at"], "2026-11-17T00:00:00Z");
    assert_eq!(edited["updated_at"], "2026-11-17T00:05:00Z");
    assert!(edited["deleted_at"].is_null());

    let (s, page) = h.list(&t, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 1, "{page}");
    assert_eq!(page["items"][0]["body"], "第二次跟进");

    let (s, revs) = h.revisions(&t, &note, "").await;
    assert_eq!(s, StatusCode::OK, "{revs}");
    let revisions = revs["items"].as_array().unwrap();
    assert_eq!(revisions.len(), 1, "{revs}");
    assert_eq!(revisions[0]["activity_id"], note["id"]);
    assert_eq!(revisions[0]["version"], 1);
    assert_eq!(revisions[0]["author_kind"], "manual");
    assert_eq!(revisions[0]["body"], "第一次跟进");
    assert_eq!(revisions[0]["created_at"], "2026-11-17T00:00:00Z");

    h.clock.advance(chrono::Duration::minutes(5));
    assert_eq!(
        h.delete_activity(&t, &note, 2).await,
        StatusCode::NO_CONTENT
    );
    // The marker stays observable: the timeline can show that a follow-up was
    // removed, while the content lives only in history.
    let (_, page) = h.list(&t, "").await;
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{page}");
    assert_eq!(items[0]["id"], note["id"]);
    assert_eq!(items[0]["version"], 3);
    assert_eq!(items[0]["body"], "", "a marker carries no content: {page}");
    assert_eq!(items[0]["deleted_at"], "2026-11-17T00:10:00Z", "{page}");
    let row: (String, Option<chrono::DateTime<chrono::Utc>>, i64) =
        sqlx::query_as("SELECT body,deleted_at,version FROM activities WHERE id=$1")
            .bind(uid(&note))
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(row.0, "", "the row is emptied, the body moved to history");
    assert!(row.1.is_some(), "a delete only leaves a marker");
    assert_eq!(row.2, 3);

    let (s, revs) = h.revisions(&t, &note, "").await;
    assert_eq!(s, StatusCode::OK, "{revs}");
    let bodies: Vec<_> = revs["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["version"].as_i64().unwrap(), r["body"].as_str().unwrap()))
        .collect();
    assert!(bodies.contains(&(1, "第一次跟进")), "{revs}");
    assert!(
        bodies.contains(&(2, "第二次跟进")),
        "deleted content is history only: {revs}"
    );
    assert_eq!(bodies.len(), 2, "{revs}");
    assert_eq!(revs["items"][0]["version"], 2);
    assert_eq!(revs["items"][0]["created_at"], "2026-11-17T00:05:00Z");

    // A deleted note is no longer editable through the live operation.
    let (s, _) = h.edit(&t, &note, 3, "再来一次").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h.revision_count(&note).await, 2);
}

// Every rejected edit path must leave the stored content and the revision count
// untouched, so no orphan revision can be read back later.
#[tokio::test]
async fn t36_3_failed_edits_leave_no_orphan_revision() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"跟进对象"})).await;
    let (_, note) = h.create(&t, "原文").await;

    let (s, err) = h
        .patch(&t, &note, json!({"expected_version":5,"body":"改一"}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "version_conflict");
    assert_eq!(err["current_version"], 1);
    assert_eq!(h.revision_count(&note).await, 0);

    let oversized = "x".repeat(10_001);
    for (body, reason) in [
        ("".to_owned(), "empty body"),
        ("\u{0}".to_owned(), "nul body"),
        (oversized, "oversized body"),
    ] {
        let (s, _) = h
            .patch(&t, &note, json!({"expected_version":1,"body":body}))
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{reason}");
        assert_eq!(h.revision_count(&note).await, 0, "{reason}");
    }

    let (s, _) = h
        .patch(&t, &note, json!({"expected_version":0,"body":"改一"}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = h.patch(&t, &note, json!({"expected_version":1})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(h.revision_count(&note).await, 0);

    // A single accepted edit writes exactly one revision, and repeating the
    // stale edit afterwards adds nothing.
    let (s, edited) = h.edit(&t, &note, 1, "第二版").await;
    assert_eq!(s, StatusCode::OK, "{edited}");
    assert_eq!(edited["version"], 2);
    assert_eq!(h.revision_count(&note).await, 1);
    let (s, _) = h.edit(&t, &note, 1, "重放旧版本").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(h.revision_count(&note).await, 1);
    let (s, revs) = h.revisions(&t, &note, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(revs["items"].as_array().unwrap().len(), 1);
    assert_eq!(revs["items"][0]["body"], "原文");

    // Wrong parent task, unknown id and a deleted task all fail without writes.
    let other = h.task(json!({"title":"另一个"})).await;
    let (s, _) = h
        .patch(&other, &note, json!({"expected_version":2,"body":"穿线"}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let missing = json!({"id":Uuid::new_v4().to_string()});
    let (s, _) = h.edit(&t, &missing, 1, "不存在").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h.revision_count(&note).await, 1);
    let (s, _, _) = h
        .send(
            "DELETE",
            &format!("/api/v1/tasks/{}?expected_version=1", uid(&t)),
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = h.edit(&t, &note, 2, "已删事项").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = h.create(&t, "已删事项备注").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h.revision_count(&note).await, 1);
}

// Cursors are opaque, scoped and validated; a mismatched cursor is a 400 rather
// than a silent page of another task's notes.
#[tokio::test]
async fn t36_4_activity_pages_are_scoped_and_validated() {
    let h = Harness::new().await;
    let a = h.task(json!({"title":"甲"})).await;
    let b = h.task(json!({"title":"乙"})).await;
    for body in ["一", "二", "三"] {
        let (s, _) = h.create(&a, body).await;
        assert_eq!(s, StatusCode::CREATED);
        h.clock.advance(chrono::Duration::minutes(1));
    }
    let (s, first) = h.list(&a, "?limit=1").await;
    assert_eq!(s, StatusCode::OK, "{first}");
    let mut seen = vec![first["items"][0]["body"].as_str().unwrap().to_owned()];
    let mut page = first.clone();
    while !page["next_cursor"].is_null() {
        let cursor = page["next_cursor"].as_str().unwrap();
        let (s, next) = h.list(&a, &format!("?limit=1&cursor={cursor}")).await;
        assert_eq!(s, StatusCode::OK, "{next}");
        seen.push(next["items"][0]["body"].as_str().unwrap().to_owned());
        page = next;
    }
    assert_eq!(seen, vec!["三", "二", "一"]);
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        h.list(&b, &format!("?limit=1&cursor={cursor}")).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(h.list(&a, "?limit=0").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(h.list(&a, "?limit=101").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        h.list(&a, "?cursor=not-a-cursor").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.list(&json!({"id":Uuid::new_v4().to_string()}), "")
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let note = first["items"][0].clone();
    h.clock.advance(chrono::Duration::minutes(1));
    h.edit(&a, &note, 1, "三改").await;
    h.clock.advance(chrono::Duration::minutes(1));
    let (_, current) = h.list(&a, "").await;
    let note = current["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["body"] == "三改")
        .unwrap()
        .clone();
    h.edit(&a, &note, 2, "三再改").await;
    let (s, revs) = h.revisions(&a, &note, "?limit=1").await;
    assert_eq!(s, StatusCode::OK, "{revs}");
    assert_eq!(revs["items"][0]["body"], "三改");
    let next = revs["next_cursor"].as_str().unwrap();
    let (s, second) = h
        .revisions(&a, &note, &format!("?limit=1&cursor={next}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{second}");
    assert_eq!(second["items"][0]["body"], "三");
    assert!(second["next_cursor"].is_null(), "{second}");
    assert_eq!(h.revisions(&b, &note, "").await.0, StatusCode::NOT_FOUND);

    // A manual note is validated on create exactly as it is on edit.
    let oversized = "y".repeat(10_001);
    for (body, reason) in [
        ("".to_owned(), "empty create"),
        ("\u{0}".to_owned(), "nul create"),
        (oversized, "oversized create"),
    ] {
        assert_eq!(
            h.create(&a, &body).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{reason}"
        );
    }
    assert_eq!(
        h.create(&json!({"id":Uuid::new_v4().to_string()}), "无事项")
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Revisions are scoped to their own activity, like the task page is scoped
    // to its own activities.
    let (_, extra) = h.create(&a, "丁").await;
    h.edit(&a, &extra, 1, "丁改").await;
    let (_, extra_revs) = h.revisions(&a, &extra, "").await;
    assert_eq!(extra_revs["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.revisions(&a, &extra, &format!("?cursor={next}")).await.0,
        StatusCode::BAD_REQUEST
    );
    // A note of another task is not deletable through this task's path.
    assert_eq!(h.delete_activity(&b, &note, 3).await, StatusCode::NOT_FOUND);

    // The paths sit behind the same session and CSRF guard as every other
    // follow-up write: an anonymous read is rejected and a cookie-only write is
    // refused, while a plain GET stays public to the session guard only.
    let path = format!("/api/v1/tasks/{}/activities", uid(&a));
    let anonymous = h
        .app
        .clone()
        .oneshot(Request::get(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let mut checks = vec![(
        Request::post(&path)
            .header("cookie", &h.cookie)
            .header("origin", AuthConfig::default().origin)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"body":"无 CSRF"}"#))
            .unwrap(),
        StatusCode::FORBIDDEN,
    )];
    checks.push((
        Request::post(&path)
            .header("origin", AuthConfig::default().origin)
            .header("x-csrf-token", &h.csrf)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"body":"无会话"}"#))
            .unwrap(),
        StatusCode::UNAUTHORIZED,
    ));
    for (request, expected) in checks {
        let r = h.app.clone().oneshot(request).await.unwrap();
        assert_eq!(r.status(), expected);
    }
}

// Follow-up writes follow the task write guards: a read still lists an archived
// task's history, but an archived task or an archived/deleted project refuses
// every new note, edit and delete.
#[tokio::test]
async fn t36_5_writes_follow_task_and_project_guards() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"归档对象"})).await;
    let (_, note) = h.create(&t, "归档前").await;
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(uid(&t))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (s, page) = h.list(&t, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 1, "{page}");
    let (s, refused) = h.create(&t, "归档后").await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert!(
        refused.get("current_version").is_none(),
        "a state conflict never reports a foreign version: {refused}"
    );
    assert_eq!(
        h.edit(&t, &note, 1, "归档后改").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.delete_activity(&t, &note, 1).await, StatusCode::CONFLICT);
    let (_, page) = h.list(&t, "").await;
    assert_eq!(page["items"][0]["body"], "归档前");
    assert_eq!(page["items"][0]["version"], 1);
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM activities WHERE task_id=$1")
        .bind(uid(&t))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(live, 1, "refused writes add no record");

    // A live task inside an archived or deleted project refuses new notes.
    let (s, _, p) = h
        .send(
            "POST",
            "/api/v1/projects",
            json!({"title":"项目","description":"d"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let c = h.task(json!({"title":"项目内","project_id":p["id"]})).await;
    for state in ["archived_at=now()", "deleted_at=now()"] {
        sqlx::query(&format!("UPDATE projects SET {state} WHERE id=$1"))
            .bind(uid(&p))
            .execute(&h.db.pool)
            .await
            .unwrap();
        assert_eq!(
            h.create(&c, "受限项目备注").await.0,
            StatusCode::CONFLICT,
            "{state}"
        );
    }
    sqlx::query("UPDATE projects SET deleted_at=NULL,archived_at=NULL WHERE id=$1")
        .bind(uid(&p))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.create(&c, "可写项目备注").await.0, StatusCode::CREATED);
    assert_eq!(h.revision_count(&note).await, 0);
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
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            sqlx::query("SELECT pg_stat_clear_snapshot()")
                .execute(&mut *c)
                .await
                .unwrap();
            let pid: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND pid IS DISTINCT FROM $2 AND $1=ANY(pg_blocking_pids(pid)) ORDER BY pid LIMIT 1",
            )
            .bind(by)
            .bind(exclude)
            .fetch_optional(&mut *c)
            .await
            .unwrap();
            if let Some(pid) = pid {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the real backend must reach a controlled wait")
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

// Two concurrent edits with the same expected version must produce one winner
// and one 409 — never a second history row for the same version and never a 503
// from the history uniqueness constraint. The gate blocks both writers before
// they admit the relation, so this characterises the lock order (relation
// advisory lock, then task row) rather than the row CAS on its own.
#[tokio::test]
async fn t36_6_concurrent_edits_serialize_without_orphan_history() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"并发"})).await;
    let (_, note) = h.create(&t, "原文").await;
    use sqlx::Acquire;
    let mut gate = gate_connection(&h).await;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut tx = gate.begin().await.unwrap();
    sqlx::query("SELECT id FROM tasks WHERE id=$1 FOR UPDATE")
        .bind(uid(&t))
        .execute(&mut *tx)
        .await
        .unwrap();
    let path = format!("/api/v1/tasks/{}/activities/{}", uid(&t), uid(&note));
    let first = tokio::spawn(h.app.clone().oneshot(request(
        &h,
        "PATCH",
        &path,
        json!({"expected_version":1,"body":"甲改"}),
    )));
    let writer = blocked(&mut tx, pid, None).await;
    let second = tokio::spawn(h.app.clone().oneshot(request(
        &h,
        "PATCH",
        &path,
        json!({"expected_version":1,"body":"乙改"}),
    )));
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
    let mut codes = [first.status(), second.status()];
    codes.sort_by_key(|s| s.as_u16());
    assert_eq!(
        codes,
        [StatusCode::OK, StatusCode::CONFLICT],
        "one edit wins, the loser is a version conflict"
    );
    assert_eq!(h.revision_count(&note).await, 1);
    let (_, page) = h.list(&t, "").await;
    let body = page["items"][0]["body"].as_str().unwrap().to_owned();
    assert!(body == "甲改" || body == "乙改", "{body}");
    assert_eq!(page["items"][0]["version"], 2);
}

// A status change appends inside the task transaction; a rejected change must
// leave neither the task nor the event behind, and non-status edits never append.
#[tokio::test]
async fn t36_7_only_committed_status_changes_append_system_events() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"原子性"})).await;
    // An injected failure inside the same transaction must roll the task back
    // with the event: the update never stands without its follow-up record.
    sqlx::raw_sql(
        "CREATE FUNCTION t36_fail_activity() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 't36 injected append failure'; END $$",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER t36_fail_activity BEFORE INSERT ON activities
         FOR EACH ROW EXECUTE FUNCTION t36_fail_activity()",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let (s, _, b) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&t)),
            json!({"expected_version":1,"status":"in_progress"}),
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    sqlx::raw_sql("DROP TRIGGER t36_fail_activity ON activities")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let row: (String, i64) = sqlx::query_as("SELECT status,version FROM tasks WHERE id=$1")
        .bind(uid(&t))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        row,
        ("todo".to_owned(), 1),
        "the task update rolled back too"
    );
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM activities WHERE task_id=$1")
        .bind(uid(&t))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);

    // Title, description and priority edits are not status events.
    for patch in [
        json!({"expected_version":1,"title":"改名"}),
        json!({"expected_version":2,"description":"说明"}),
        json!({"expected_version":3,"priority":"high"}),
    ] {
        let (s, _, b) = h
            .send("PATCH", &format!("/api/v1/tasks/{}", uid(&t)), patch)
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let (_, page) = h.list(&t, "").await;
    assert_eq!(
        page["items"].as_array().unwrap().len(),
        0,
        "non-status edits append nothing: {page}"
    );
    let (s, _, _) = h
        .send(
            "PATCH",
            &format!("/api/v1/tasks/{}", uid(&t)),
            json!({"expected_version":4,"status":"done"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, page) = h.list(&t, "").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1, "{page}");
    assert_eq!(page["items"][0]["body"], "status: todo -> done");
}
