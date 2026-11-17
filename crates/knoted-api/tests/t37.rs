//! T37: real HTTP/PG lifecycle, readonly admission and safe task-only recycling.
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
    tasks::lifecycle::{self, PurgeTrash},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

struct Harness {
    db: TestDb,
    ctx: AppContext,
    clock: FixedClock,
    app: Router,
    cookie: String,
    csrf: String,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let clock = FixedClock::new("2026-11-17T00:00:00Z".parse().unwrap());
        let ctx = AppContext::new(db.pool.clone()).with_clock(Arc::new(clock.clone()));
        let mut tx = db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let s = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        Self {
            app: build_router_with_auth(ctx.clone(), AuthConfig::default()),
            cookie: format!("knoted_session={}; knoted_csrf={}", s.token, s.csrf_token),
            csrf: s.csrf_token,
            ctx,
            clock,
            db,
        }
    }
    async fn refresh_session(&mut self) {
        let mut tx = self.db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let s = sessions::create_session(&mut tx, owner.user.id, self.ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        self.cookie = format!("knoted_session={}; knoted_csrf={}", s.token, s.csrf_token);
        self.csrf = s.csrf_token;
    }
    async fn send(&self, method: &str, path: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
        self.send_auth(method, path, body, true, true).await
    }
    async fn send_auth(
        &self,
        method: &str,
        path: &str,
        body: Value,
        auth: bool,
        csrf: bool,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if auth {
            req = req.header("cookie", &self.cookie);
        }
        if csrf {
            req = req
                .header("origin", AuthConfig::default().origin)
                .header("x-csrf-token", &self.csrf);
        }
        let r = self
            .app
            .clone()
            .oneshot(
                req.body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(serde_json::to_vec(&body).unwrap())
                })
                .unwrap(),
            )
            .await
            .unwrap();
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
    async fn expect(&self, method: &str, path: &str, body: Value, status: StatusCode) -> Value {
        let (s, _, b) = self.send(method, path, body).await;
        assert_eq!(s, status, "{method} {path}: {b}");
        b
    }
    async fn task(&self, body: Value) -> Value {
        self.expect("POST", "/api/v1/tasks", body, StatusCode::CREATED)
            .await
    }
    async fn project(&self) -> Value {
        self.expect(
            "POST",
            "/api/v1/projects",
            json!({"title":"项目"}),
            StatusCode::CREATED,
        )
        .await
    }
    async fn action(&self, t: &Value, action: &str, v: i64, status: StatusCode) -> Value {
        self.expect(
            "POST",
            &format!("{}/{action}", path(t)),
            json!({"expected_version":v}),
            status,
        )
        .await
    }
    async fn delete(&self, t: &Value, v: i64, status: StatusCode) {
        self.expect(
            "DELETE",
            &format!("{}?expected_version={v}", path(t)),
            Value::Null,
            status,
        )
        .await;
    }
    async fn patch(&self, t: &Value, body: Value, status: StatusCode) -> Value {
        self.expect("PATCH", &path(t), body, status).await
    }
    // Persist associations before freezing the source; their removal is also a write.
    async fn relations(&self, t: &Value) -> (Value, Value, Value) {
        let activity = self
            .expect(
                "POST",
                &format!("{}/activities", path(t)),
                json!({"body":"原跟进"}),
                StatusCode::CREATED,
            )
            .await;
        let tag = self
            .expect(
                "POST",
                "/api/v1/tags",
                json!({"name":format!("tag-{}", uid(t))}),
                StatusCode::CREATED,
            )
            .await;
        self.expect(
            "PUT",
            &format!("/api/v1/entities/task/{}/tags/{}", uid(t), uid(&tag)),
            Value::Null,
            StatusCode::NO_CONTENT,
        )
        .await;
        let target = self.task(json!({"title":"关联目标"})).await;
        let link = self.expect("POST", "/api/v1/links", json!({"source_type":"task","source_id":t["id"],"target_type":"task","target_id":target["id"]}), StatusCode::CREATED).await;
        (activity, tag, link)
    }
    async fn readonly(&self, t: &Value, v: i64, a: &Value, tag: &Value, link: &Value) {
        let err = self
            .patch(
                t,
                json!({"expected_version":v,"title":"不应写入"}),
                StatusCode::CONFLICT,
            )
            .await;
        assert_eq!(err["current_version"], v);
        for (method, url, body) in [
            (
                "POST",
                format!("{}/activities", path(t)),
                json!({"body":"不应追加"}),
            ),
            (
                "PATCH",
                format!("{}/activities/{}", path(t), uid(a)),
                json!({"expected_version":1,"body":"不应修改"}),
            ),
            (
                "DELETE",
                format!("{}/activities/{}?expected_version=1", path(t), uid(a)),
                Value::Null,
            ),
            (
                "PUT",
                format!("/api/v1/entities/task/{}/tags/{}", uid(t), uid(tag)),
                Value::Null,
            ),
            (
                "DELETE",
                format!("/api/v1/entities/task/{}/tags/{}", uid(t), uid(tag)),
                Value::Null,
            ),
            (
                "POST",
                "/api/v1/links".into(),
                json!({"source_type":"task","source_id":t["id"],"target_type":"task","target_id":link["target_id"]}),
            ),
            (
                "DELETE",
                format!("/api/v1/links/{}?expected_version=1", uid(link)),
                Value::Null,
            ),
        ] {
            let b = self.expect(method, &url, body, StatusCode::CONFLICT).await;
            assert_eq!(b["code"], "version_conflict");
            assert!(
                b.get("current_version").is_none(),
                "state refusal is not an activity/link CAS: {b}"
            );
        }
        for url in [
            path(t),
            format!("{}/activities", path(t)),
            format!("{}/activities/{}/revisions", path(t), uid(a)),
            format!("/api/v1/entities/task/{}/tags", uid(t)),
            format!("/api/v1/links?source_type=task&source_id={}", uid(t)),
        ] {
            self.expect("GET", &url, Value::Null, StatusCode::OK).await;
        }
    }
    async fn send_tcp(
        &self,
        address: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, String, Value) {
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        let request = format!(
            "{method} {path} HTTP/1.0\r\nHost: {address}\r\nConnection: close\r\nCookie: {}\r\nOrigin: {}\r\nX-CSRF-Token: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.cookie,
            AuthConfig::default().origin,
            self.csrf,
            body.len()
        );
        let response = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
        .await
        .unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status =
            StatusCode::from_u16(head.split_whitespace().nth(1).unwrap().parse().unwrap()).unwrap();
        (
            status,
            head.to_ascii_lowercase(),
            if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(body).unwrap()
            },
        )
    }
    async fn task_row(&self, t: &Value) -> Value {
        sqlx::query_scalar("SELECT row_to_json(t) FROM tasks t WHERE id=$1")
            .bind(uid(t))
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
    async fn count(&self, sql: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
    async fn purge(&self) -> u64 {
        lifecycle::purge_trash(&self.ctx, PurgeTrash {})
            .await
            .unwrap()
            .purged
    }
    async fn outbox(&self, t: &Value) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM outbox WHERE topic='task.changed' AND payload->>'source_id'=$1",
        )
        .bind(uid(t).to_string())
        .fetch_one(&self.db.pool)
        .await
        .unwrap()
    }
}
fn uid(t: &Value) -> Uuid {
    t["id"].as_str().unwrap().parse().unwrap()
}
fn path(t: &Value) -> String {
    format!("/api/v1/tasks/{}", uid(t))
}

// Detects missing lifecycle registration, version/outbox changes, or any archived write bypass.
#[tokio::test]
async fn t37_1_archive_is_readonly_and_unarchive_restores_writes() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"未完成也能独立归档"})).await;
    let (a, tag, link) = h.relations(&t).await;
    let (s, headers, archived) = h
        .send(
            "POST",
            &format!("{}/archive", path(&t)),
            json!({"expected_version":1}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{archived}");
    assert_eq!(headers["etag"], "\"2\"");
    assert_eq!(archived["version"], 2);
    assert!(!archived["archived_at"].is_null());
    assert_eq!(h.outbox(&t).await, 2);
    h.readonly(&t, 2, &a, &tag, &link).await;
    h.action(&t, "archive", 2, StatusCode::CONFLICT).await;
    h.action(&t, "unarchive", 1, StatusCode::CONFLICT).await;
    h.delete(&t, 2, StatusCode::CONFLICT).await;
    let unarchived = h.action(&t, "unarchive", 2, StatusCode::OK).await;
    assert_eq!(unarchived["version"], 3);
    assert!(unarchived["archived_at"].is_null());
    h.patch(
        &t,
        json!({"expected_version":3,"title":"可写"}),
        StatusCode::OK,
    )
    .await;
    h.expect(
        "POST",
        &format!("{}/activities", path(&t)),
        json!({"body":"恢复写入"}),
        StatusCode::CREATED,
    )
    .await;
    h.expect(
        "DELETE",
        &format!("/api/v1/links/{}?expected_version=1", uid(&link)),
        Value::Null,
        StatusCode::NO_CONTENT,
    )
    .await;
    h.expect(
        "DELETE",
        &format!("/api/v1/entities/task/{}/tags/{}", uid(&t), uid(&tag)),
        Value::Null,
        StatusCode::NO_CONTENT,
    )
    .await;
}

#[tokio::test]
async fn t37_1_parent_archive_is_local_and_child_unarchive_checks_parent() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"非终态父"})).await;
    let c = h
        .task(json!({"title":"非终态子","parent_id":p["id"]}))
        .await;
    h.action(&p, "archive", 1, StatusCode::OK).await;
    let child = h
        .expect("GET", &path(&c), Value::Null, StatusCode::OK)
        .await;
    assert!(child["archived_at"].is_null());
    assert_eq!(child["version"], 1);
    assert_eq!(h.outbox(&c).await, 1);
    h.action(&p, "unarchive", 2, StatusCode::OK).await;
    h.action(&c, "archive", 1, StatusCode::OK).await;
    h.action(&p, "archive", 3, StatusCode::OK).await;
    h.action(&c, "unarchive", 2, StatusCode::CONFLICT).await;
    h.action(&p, "unarchive", 4, StatusCode::OK).await;
    h.action(&c, "unarchive", 2, StatusCode::OK).await;
}

// Direct PG fixtures isolate the missing guards from the missing archive route.
#[tokio::test]
async fn t37_1_parent_readonly_seals_every_child_write_but_not_reads() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"父"})).await;
    let t = h.task(json!({"title":"子","parent_id":p["id"]})).await;
    let (a, tag, link) = h.relations(&t).await;
    sqlx::query("UPDATE tasks SET archived_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.readonly(&t, 1, &a, &tag, &link).await;
    h.action(&t, "archive", 1, StatusCode::CONFLICT).await;
    h.delete(&t, 1, StatusCode::CONFLICT).await;
    // Parent archive is local, never a child cascade.
    let child = h
        .expect("GET", &path(&t), Value::Null, StatusCode::OK)
        .await;
    assert!(child["archived_at"].is_null());
    assert_eq!(child["version"], 1);
    sqlx::query("UPDATE tasks SET archived_at=NULL,deleted_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.readonly(&t, 1, &a, &tag, &link).await;
}

#[tokio::test]
async fn t37_1_project_readonly_seals_tags_links_and_activities() {
    let h = Harness::new().await;
    let p = h.project().await;
    let t = h.task(json!({"title":"事项","project_id":p["id"]})).await;
    let (a, tag, link) = h.relations(&t).await;
    sqlx::query("UPDATE projects SET archived_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.readonly(&t, 1, &a, &tag, &link).await;
    h.action(&t, "archive", 1, StatusCode::CONFLICT).await;
    sqlx::query("UPDATE projects SET archived_at=NULL,deleted_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.readonly(&t, 1, &a, &tag, &link).await;
}

#[tokio::test]
async fn t37_1_lifecycle_http_validation_and_auth() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"动作参数"})).await;
    for action in ["archive", "unarchive", "restore"] {
        let url = format!("{}/{action}", path(&t));
        let b = json!({"expected_version":1});
        assert_eq!(
            h.send_auth("POST", &url, b.clone(), false, false).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            h.send_auth("POST", &url, b, true, false).await.0,
            StatusCode::FORBIDDEN
        );
        for body in [
            json!({}),
            json!({"expected_version":null}),
            json!({"expected_version":"1"}),
            json!({"expected_version":1,"extra":true}),
        ] {
            h.expect("POST", &url, body, StatusCode::BAD_REQUEST).await;
        }
        h.expect(
            "POST",
            &url,
            json!({"expected_version":0}),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
        h.expect(
            "POST",
            &format!("/api/v1/tasks/not-a-uuid/{action}"),
            json!({"expected_version":1}),
            StatusCode::BAD_REQUEST,
        )
        .await;
        h.expect(
            "POST",
            &format!("/api/v1/tasks/{}/{action}", Uuid::new_v4()),
            json!({"expected_version":1}),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    h.action(&t, "unarchive", 1, StatusCode::CONFLICT).await;
    h.action(&t, "restore", 1, StatusCode::CONFLICT).await;
}

// Detects lost content/relations, a hard 30-day restore cutoff, or missing visibility withdrawal.
#[tokio::test]
async fn t37_2_restore_after_29_days_preserves_relations_and_increments_version() {
    let mut h = Harness::new().await;
    let t = h
        .task(json!({"title":"保留","description":"正文","status":"in_progress"}))
        .await;
    let (a, tag, link) = h.relations(&t).await;
    h.expect(
        "PATCH",
        &format!("{}/activities/{}", path(&t), uid(&a)),
        json!({"expected_version":1,"body":"新版跟进"}),
        StatusCode::OK,
    )
    .await;
    h.delete(&t, 1, StatusCode::NO_CONTENT).await;
    for url in [
        path(&t),
        format!("{}/activities", path(&t)),
        format!("/api/v1/entities/task/{}/tags", uid(&t)),
        format!("/api/v1/links?source_type=task&source_id={}", uid(&t)),
    ] {
        h.expect("GET", &url, Value::Null, StatusCode::NOT_FOUND)
            .await;
    }
    let page = h
        .expect("GET", "/api/v1/tasks", Value::Null, StatusCode::OK)
        .await;
    assert!(
        !page["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == t["id"])
    );
    h.clock.advance(chrono::Duration::days(29));
    h.refresh_session().await;
    let err = h.action(&t, "restore", 1, StatusCode::CONFLICT).await;
    assert_eq!(err["current_version"], 2);
    let (s, headers, restored) = h
        .send(
            "POST",
            &format!("{}/restore", path(&t)),
            json!({"expected_version":2}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(headers["etag"], "\"3\"");
    assert_eq!(restored["version"], 3);
    for field in [
        "title",
        "description",
        "status",
        "parent_id",
        "project_id",
        "created_at",
    ] {
        assert_eq!(restored[field], t[field]);
    }
    assert!(restored["deleted_at"].is_null());
    let page = h
        .expect(
            "GET",
            &format!("{}/activities", path(&t)),
            Value::Null,
            StatusCode::OK,
        )
        .await;
    assert_eq!(page["items"][0]["body"], "新版跟进");
    let page = h
        .expect(
            "GET",
            &format!("{}/activities/{}/revisions", path(&t), uid(&a)),
            Value::Null,
            StatusCode::OK,
        )
        .await;
    assert_eq!(page["items"][0]["body"], "原跟进");
    let page = h
        .expect(
            "GET",
            &format!("/api/v1/entities/task/{}/tags", uid(&t)),
            Value::Null,
            StatusCode::OK,
        )
        .await;
    assert_eq!(page["items"][0]["id"], tag["id"]);
    let page = h
        .expect(
            "GET",
            &format!("/api/v1/links?source_type=task&source_id={}", uid(&t)),
            Value::Null,
            StatusCode::OK,
        )
        .await;
    assert_eq!(page["items"][0]["id"], link["id"]);
    assert_eq!(h.outbox(&t).await, 3);
    // Eligibility is not a restore deadline: until purge wins, an older row can be restored.
    h.delete(&t, 3, StatusCode::NO_CONTENT).await;
    h.clock.advance(chrono::Duration::days(31));
    h.refresh_session().await;
    let restored = h.action(&t, "restore", 4, StatusCode::OK).await;
    assert_eq!(restored["version"], 5);
}

#[tokio::test]
async fn t37_3_restore_parent_before_child_and_recheck_done_invariant() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"父"})).await;
    let c = h.task(json!({"title":"子","parent_id":p["id"]})).await;
    h.delete(&p, 1, StatusCode::CONFLICT).await;
    h.delete(&c, 1, StatusCode::NO_CONTENT).await;
    h.delete(&p, 1, StatusCode::NO_CONTENT).await;
    let err = h.action(&c, "restore", 2, StatusCode::CONFLICT).await;
    assert_eq!(err["current_version"], 2);
    h.action(&p, "restore", 2, StatusCode::OK).await;
    let restored = h.action(&c, "restore", 2, StatusCode::OK).await;
    assert_eq!(restored["parent_id"], p["id"]);
    h.delete(&c, 3, StatusCode::NO_CONTENT).await;
    h.patch(
        &p,
        json!({"expected_version":3,"status":"done"}),
        StatusCode::OK,
    )
    .await;
    h.action(&c, "restore", 4, StatusCode::CONFLICT).await;
    // Refusal leaves the child in trash and does not implicitly reopen the parent.
    let parent = h
        .expect("GET", &path(&p), Value::Null, StatusCode::OK)
        .await;
    assert_eq!(parent["status"], "done");
    assert_eq!(parent["version"], 4);
    assert_eq!(h.outbox(&c).await, 4);
    h.patch(
        &p,
        json!({"expected_version":4,"status":"in_progress"}),
        StatusCode::OK,
    )
    .await;
    h.action(&c, "restore", 4, StatusCode::OK).await;
}

#[tokio::test]
async fn t37_3_restore_uses_current_migrated_project_and_rejects_readonly_relations() {
    let h = Harness::new().await;
    let p1 = h.project().await;
    let p2 = h.project().await;
    let p = h.task(json!({"title":"父","project_id":p1["id"]})).await;
    let c = h.task(json!({"title":"子","parent_id":p["id"]})).await;
    h.delete(&c, 1, StatusCode::NO_CONTENT).await;
    h.patch(
        &p,
        json!({"expected_version":1,"project_id":p2["id"]}),
        StatusCode::OK,
    )
    .await;
    let err = h.action(&c, "restore", 2, StatusCode::CONFLICT).await;
    assert_eq!(err["current_version"], 3);
    // Every invalid current relation must be a 409, with neither version nor outbox changes.
    for statement in [
        "UPDATE projects SET archived_at=now() WHERE id=$1",
        "UPDATE projects SET deleted_at=now() WHERE id=$1",
    ] {
        sqlx::query(statement)
            .bind(uid(&p2))
            .execute(&h.db.pool)
            .await
            .unwrap();
        h.action(&c, "restore", 3, StatusCode::CONFLICT).await;
        sqlx::query("UPDATE projects SET archived_at=NULL,deleted_at=NULL WHERE id=$1")
            .bind(uid(&p2))
            .execute(&h.db.pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE tasks SET archived_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.action(&c, "restore", 3, StatusCode::CONFLICT).await;
    sqlx::query("UPDATE tasks SET archived_at=NULL WHERE id=$1")
        .bind(uid(&p))
        .execute(&h.db.pool)
        .await
        .unwrap();
    // Historical inheritance mismatch is not silently repaired.
    sqlx::query("UPDATE tasks SET project_id=$2 WHERE id=$1")
        .bind(uid(&c))
        .bind(uid(&p1))
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.action(&c, "restore", 3, StatusCode::CONFLICT).await;
    sqlx::query("UPDATE tasks SET project_id=$2 WHERE id=$1")
        .bind(uid(&c))
        .bind(uid(&p2))
        .execute(&h.db.pool)
        .await
        .unwrap();
    let grandparent = h
        .task(json!({"title":"历史异常祖父","project_id":p2["id"]}))
        .await;
    sqlx::query("UPDATE tasks SET parent_id=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(uid(&grandparent))
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.action(&c, "restore", 3, StatusCode::CONFLICT).await;
    sqlx::query("UPDATE tasks SET parent_id=NULL WHERE id=$1")
        .bind(uid(&p))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.outbox(&c).await, 3);
    let restored = h.action(&c, "restore", 3, StatusCode::OK).await;
    assert_eq!(restored["parent_id"], p["id"]);
    assert_eq!(restored["project_id"], p2["id"]);
    assert_eq!(restored["version"], 4);
}

// Detects early expiry, parent-first deletion, accidental identity/link/tag/outbox removal,
// or forgetting the dependent timeline rows (including immutable system events).
#[tokio::test]
async fn t37_4_purge_at_30_days_is_task_only_leaf_first_and_idempotent() {
    let h = Harness::new().await;
    let project = h.project().await;
    let p = h.task(json!({"title":"父"})).await;
    let c = h.task(json!({"title":"子","parent_id":p["id"]})).await;
    let (a, tag, link) = h.relations(&c).await;
    h.patch(
        &c,
        json!({"expected_version":1,"status":"in_progress"}),
        StatusCode::OK,
    )
    .await;
    h.expect(
        "PATCH",
        &format!("{}/activities/{}", path(&c), uid(&a)),
        json!({"expected_version":1,"body":"新版"}),
        StatusCode::OK,
    )
    .await;
    let inbound = h.expect("POST", "/api/v1/links", json!({"source_type":"task","source_id":link["target_id"],"target_type":"task","target_id":c["id"]}), StatusCode::CREATED).await;
    let note = h
        .expect(
            "POST",
            "/api/v1/notes",
            json!({"title":"非事项","document":{"blocks":[]}}),
            StatusCode::CREATED,
        )
        .await;
    let restored = h.task(json!({"title":"已恢复不清理"})).await;
    h.delete(&restored, 1, StatusCode::NO_CONTENT).await;
    h.action(&restored, "restore", 2, StatusCode::OK).await;
    h.delete(&c, 2, StatusCode::NO_CONTENT).await;
    h.delete(&p, 1, StatusCode::NO_CONTENT).await;
    // Other lifecycle owners are deliberately outside task purge.
    sqlx::query("UPDATE projects SET deleted_at=$2 WHERE id=$1")
        .bind(uid(&project))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE notes SET deleted_at=$2 WHERE id=$1")
        .bind(uid(&note))
        .bind(h.ctx.now())
        .execute(&h.db.pool)
        .await
        .unwrap();
    let outbox = h.outbox(&c).await;
    h.clock.advance(chrono::Duration::days(29));
    assert_eq!(h.purge().await, 0);
    assert_eq!(
        h.count("SELECT count(*) FROM tasks WHERE id=$1", uid(&c))
            .await,
        1
    );
    h.clock
        .advance(chrono::Duration::days(1) - chrono::Duration::microseconds(1));
    assert_eq!(h.purge().await, 0);
    h.clock.advance(chrono::Duration::microseconds(1));
    assert_eq!(h.purge().await, 2);
    for id in [uid(&p), uid(&c)] {
        assert_eq!(
            h.count("SELECT count(*) FROM tasks WHERE id=$1", id).await,
            0
        );
        assert_eq!(
            h.count("SELECT count(*) FROM entity_registry WHERE id=$1", id)
                .await,
            1
        );
        assert_eq!(
            h.count("SELECT count(*) FROM activities WHERE task_id=$1", id)
                .await,
            0
        );
        assert_eq!(
            h.count("SELECT count(*) FROM task_tags WHERE entity_id=$1", id)
                .await,
            0
        );
    }
    assert_eq!(
        h.count(
            "SELECT count(*) FROM activity_revisions WHERE activity_id=$1",
            uid(&a)
        )
        .await,
        0
    );
    for id in [uid(&link), uid(&inbound)] {
        assert_eq!(
            h.count("SELECT count(*) FROM entity_links WHERE id=$1", id)
                .await,
            1
        );
    }
    assert_eq!(
        h.count("SELECT count(*) FROM tags WHERE id=$1", uid(&tag))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM tasks WHERE id=$1", uid(&restored))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM projects WHERE id=$1", uid(&project))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM notes WHERE id=$1", uid(&note))
            .await,
        1
    );
    assert_eq!(h.outbox(&c).await, outbox);
    assert_eq!(h.purge().await, 0);
}

#[tokio::test]
async fn t37_4_young_or_live_child_blocks_old_parent_without_detachment() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"老父"})).await;
    let c = h.task(json!({"title":"年轻子","parent_id":p["id"]})).await;
    h.delete(&c, 1, StatusCode::NO_CONTENT).await;
    h.delete(&p, 1, StatusCode::NO_CONTENT).await;
    // Historical timestamps need not sort with the tree: eligibility is per row.
    sqlx::query("UPDATE tasks SET deleted_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now() - chrono::Duration::days(31))
        .execute(&h.db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET deleted_at=$2 WHERE id=$1")
        .bind(uid(&c))
        .bind(h.ctx.now() - chrono::Duration::days(29))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.purge().await, 0);
    let parent: Option<Uuid> = sqlx::query_scalar("SELECT parent_id FROM tasks WHERE id=$1")
        .bind(uid(&c))
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(parent, Some(uid(&p)));
    h.clock.advance(chrono::Duration::days(1));
    assert_eq!(h.purge().await, 2);
    // Even a historically live child must block physical cleanup of its deleted parent.
    let p = h.task(json!({"title":"异常老父"})).await;
    let c = h.task(json!({"title":"活子","parent_id":p["id"]})).await;
    sqlx::query("UPDATE tasks SET deleted_at=$2 WHERE id=$1")
        .bind(uid(&p))
        .bind(h.ctx.now() - chrono::Duration::days(31))
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.purge().await, 0);
    assert_eq!(
        h.count("SELECT count(*) FROM tasks WHERE id=$1", uid(&p))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM tasks WHERE parent_id=$1", uid(&p))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM tasks WHERE id=$1", uid(&c))
            .await,
        1
    );
}

#[tokio::test]
async fn t37_4_purge_failure_rolls_back_all_rows_and_dependencies() {
    let h = Harness::new().await;
    let p = h.task(json!({"title":"父"})).await;
    let c = h.task(json!({"title":"子","parent_id":p["id"]})).await;
    let (a, tag, _) = h.relations(&c).await;
    h.expect(
        "PATCH",
        &format!("{}/activities/{}", path(&c), uid(&a)),
        json!({"expected_version":1,"body":"修订"}),
        StatusCode::OK,
    )
    .await;
    h.delete(&c, 1, StatusCode::NO_CONTENT).await;
    h.delete(&p, 1, StatusCode::NO_CONTENT).await;
    h.clock.advance(chrono::Duration::days(30));
    // Fail only on the parent: the leaf and its dependencies have already been removed.
    sqlx::raw_sql(&format!("CREATE FUNCTION reject_purge() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.id='{}'::uuid THEN RAISE EXCEPTION 'test purge rejection'; END IF; RETURN OLD; END $$; CREATE TRIGGER reject_purge BEFORE DELETE ON tasks FOR EACH ROW EXECUTE FUNCTION reject_purge()", uid(&p))).execute(&h.db.pool).await.unwrap();
    assert!(matches!(
        lifecycle::purge_trash(&h.ctx, PurgeTrash {}).await,
        Err(knoted_core::tasks::service::DomainError::Database(_))
    ));
    for id in [uid(&p), uid(&c)] {
        assert_eq!(
            h.count("SELECT count(*) FROM tasks WHERE id=$1", id).await,
            1
        );
    }
    assert_eq!(
        h.count("SELECT count(*) FROM activities WHERE task_id=$1", uid(&c))
            .await,
        1
    );
    assert_eq!(
        h.count(
            "SELECT count(*) FROM activity_revisions WHERE activity_id=$1",
            uid(&a)
        )
        .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM task_tags WHERE entity_id=$1", uid(&c))
            .await,
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM tags WHERE id=$1", uid(&tag))
            .await,
        1
    );
    sqlx::query("DROP TRIGGER reject_purge ON tasks")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(h.purge().await, 2);
}

// Exercise the initialized server over a real TCP listener, without deploying
// or touching a running instance. Regression verification, not a claimed RED.
#[tokio::test]
async fn t37_6_tcp_server_exposes_lifecycle_with_auth_and_etags() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"TCP 生命周期"})).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = path(&t);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app = h.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    for (action, expected, next) in [
        ("archive", 1, 2),
        ("unarchive", 2, 3),
        ("delete", 3, 4),
        ("restore", 4, 5),
    ] {
        let (method, url, body) = if action == "delete" {
            (
                "DELETE",
                format!("{base}?expected_version={expected}"),
                Value::Null,
            )
        } else {
            (
                "POST",
                format!("{base}/{action}"),
                json!({"expected_version":expected}),
            )
        };
        let (status, headers, body) = h.send_tcp(address, method, &url, body).await;
        if action == "delete" {
            assert_eq!(status, StatusCode::NO_CONTENT);
        } else {
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(headers.contains(&format!("etag: \"{next}\"")));
            assert_eq!(body["version"], next);
        }
        let (status, _, _) = h.send_tcp(address, "GET", &base, Value::Null).await;
        assert_eq!(
            status,
            if action == "delete" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            }
        );
    }
    stop.send(()).unwrap();
    server.await.unwrap();
}

// Regression checks are not claimed as RED: existing CAS/outbox conventions
// must also protect each newly introduced lifecycle action.
#[tokio::test]
async fn t37_5_lifecycle_outbox_failure_rolls_back_state_version_and_relations() {
    let h = Harness::new().await;
    let t = h.task(json!({"title":"原子动作"})).await;
    let (a, _, link) = h.relations(&t).await;
    for (action, operation, version) in [
        ("archive", "archived", 1),
        ("unarchive", "unarchived", 2),
        ("delete", "deleted", 3),
        ("restore", "restored", 4),
    ] {
        let before = h.task_row(&t).await;
        let events = h.outbox(&t).await;
        sqlx::raw_sql(&format!("CREATE OR REPLACE FUNCTION reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.topic='task.changed' AND NEW.payload->>'source_id'='{}' AND NEW.payload->>'operation'='{operation}' THEN RAISE EXCEPTION 'test event rejection'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_event BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_event()", uid(&t))).execute(&h.db.pool).await.unwrap();
        if action == "delete" {
            h.delete(&t, version, StatusCode::SERVICE_UNAVAILABLE).await;
        } else {
            h.action(&t, action, version, StatusCode::SERVICE_UNAVAILABLE)
                .await;
        }
        assert_eq!(h.task_row(&t).await, before);
        assert_eq!(h.outbox(&t).await, events);
        assert_eq!(
            h.count("SELECT count(*) FROM activities WHERE id=$1", uid(&a))
                .await,
            1
        );
        assert_eq!(
            h.count("SELECT count(*) FROM entity_links WHERE id=$1", uid(&link))
                .await,
            1
        );
        sqlx::query("DROP TRIGGER reject_event ON outbox")
            .execute(&h.db.pool)
            .await
            .unwrap();
        if action == "delete" {
            h.delete(&t, version, StatusCode::NO_CONTENT).await;
        } else {
            h.action(&t, action, version, StatusCode::OK).await;
        }
    }
    assert_eq!(h.outbox(&t).await, 5);
}

#[tokio::test]
async fn t37_5_project_unarchive_never_cancels_manual_task_archives() {
    let h = Harness::new().await;
    let project = h.project().await;
    let manual = h
        .task(json!({"title":"独立归档","status":"done","project_id":project["id"]}))
        .await;
    let batch = h
        .task(json!({"title":"批次归档","status":"cancelled","project_id":project["id"]}))
        .await;
    h.action(&manual, "archive", 1, StatusCode::OK).await;
    h.expect(
        "POST",
        &format!("/api/v1/projects/{}/archive", uid(&project)),
        json!({"expected_version":1}),
        StatusCode::OK,
    )
    .await;
    let m = h.task_row(&manual).await;
    let b = h.task_row(&batch).await;
    assert!(m["archive_batch_id"].is_null());
    assert!(!b["archive_batch_id"].is_null());
    // Independent unarchive cannot bypass an archived project's readonly state.
    h.action(&manual, "unarchive", 2, StatusCode::CONFLICT)
        .await;
    h.action(&batch, "unarchive", 2, StatusCode::CONFLICT).await;
    h.expect(
        "POST",
        &format!("/api/v1/projects/{}/unarchive", uid(&project)),
        json!({"expected_version":2}),
        StatusCode::OK,
    )
    .await;
    assert_eq!(h.task_row(&manual).await, m);
    let b = h.task_row(&batch).await;
    assert!(b["archived_at"].is_null());
    assert!(b["archive_batch_id"].is_null());
    assert_eq!(b["version"], 3);
    h.action(&manual, "unarchive", 2, StatusCode::OK).await;
    // A subsequent manual archive must not pick up any old batch identity.
    h.action(&batch, "archive", 3, StatusCode::OK).await;
    h.expect(
        "POST",
        &format!("/api/v1/projects/{}/archive", uid(&project)),
        json!({"expected_version":3}),
        StatusCode::OK,
    )
    .await;
    h.expect(
        "POST",
        &format!("/api/v1/projects/{}/unarchive", uid(&project)),
        json!({"expected_version":4}),
        StatusCode::OK,
    )
    .await;
    let b = h.task_row(&batch).await;
    assert!(!b["archived_at"].is_null());
    assert_eq!(b["version"], 4);
}

#[tokio::test]
async fn t37_5_archive_and_patch_cas_races_have_one_winner_and_one_event() {
    let h = Harness::new().await;
    for race_patch in [false, true] {
        let t = h.task(json!({"title":"竞争"})).await;
        let url = format!("{}/archive", path(&t));
        let second = if race_patch { path(&t) } else { url.clone() };
        let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                h.send("POST", &url, json!({"expected_version":1})),
                h.send(
                    if race_patch { "PATCH" } else { "POST" },
                    &second,
                    if race_patch {
                        json!({"expected_version":1,"title":"竞争更新"})
                    } else {
                        json!({"expected_version":1})
                    }
                )
            )
        })
        .await
        .expect("CAS writers deadlocked");
        let statuses = [a.0, b.0];
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
        assert_eq!(
            statuses
                .iter()
                .filter(|s| **s == StatusCode::CONFLICT)
                .count(),
            1
        );
        let loser = if a.0 == StatusCode::CONFLICT {
            a.2
        } else {
            b.2
        };
        assert_eq!(loser["current_version"], 2);
        assert_eq!(h.task_row(&t).await["version"], 2);
        assert_eq!(h.outbox(&t).await, 2);
    }
}

#[tokio::test]
async fn t37_5_parent_archive_competes_with_child_activity_tag_and_reciprocal_link() {
    let h = Harness::new().await;
    for write in ["activity", "tag", "link"] {
        let p = h.task(json!({"title":"竞争父"})).await;
        let c = h.task(json!({"title":"竞争子","parent_id":p["id"]})).await;
        let (a, tag, link) = h.relations(&c).await;
        let archive_url = format!("{}/archive", path(&p));
        let (method, url, body, success) = match write {
            "activity" => (
                "POST",
                format!("{}/activities", path(&c)),
                json!({"body":"并发跟进"}),
                StatusCode::CREATED,
            ),
            "tag" => (
                "DELETE",
                format!("/api/v1/entities/task/{}/tags/{}", uid(&c), uid(&tag)),
                Value::Null,
                StatusCode::NO_CONTENT,
            ),
            _ => (
                "POST",
                "/api/v1/links".into(),
                json!({"source_type":"task","source_id":c["id"],"target_type":"task","target_id":p["id"]}),
                StatusCode::CREATED,
            ),
        };
        let (archive, write) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                h.send("POST", &archive_url, json!({"expected_version":1})),
                h.send(method, &url, body)
            )
        })
        .await
        .expect("relation writers deadlocked");
        assert_eq!(archive.0, StatusCode::OK, "{:?}", archive.2);
        assert!(
            write.0 == success || write.0 == StatusCode::CONFLICT,
            "{:?}",
            write.2
        );
        h.readonly(&c, 1, &a, &tag, &link).await;
    }
}

#[tokio::test]
async fn t37_5_restore_and_purge_are_serializable_and_never_leave_orphans() {
    let h = Harness::new().await;
    for _ in 0..6 {
        let t = h.task(json!({"title":"回收竞争"})).await;
        let (a, _, link) = h.relations(&t).await;
        h.delete(&t, 1, StatusCode::NO_CONTENT).await;
        sqlx::query("UPDATE tasks SET deleted_at=$2 WHERE id=$1")
            .bind(uid(&t))
            .bind(h.ctx.now() - chrono::Duration::days(30))
            .execute(&h.db.pool)
            .await
            .unwrap();
        let (purge, restore) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                lifecycle::purge_trash(&h.ctx, PurgeTrash {}),
                lifecycle::restore(
                    &h.ctx,
                    lifecycle::RestoreTask {
                        id: uid(&t),
                        expected_version: 2
                    }
                )
            )
        })
        .await
        .expect("purge/restore deadlocked");
        let report = purge.unwrap();
        match restore {
            Ok(restored) => {
                assert_eq!(report.purged, 0);
                assert!(restored.deleted_at.is_none());
                assert_eq!(restored.version, 3);
                assert_eq!(
                    h.count("SELECT count(*) FROM activities WHERE id=$1", uid(&a))
                        .await,
                    1
                );
                assert_eq!(h.outbox(&t).await, 3);
            }
            Err(knoted_core::tasks::service::DomainError::NotFound) => {
                assert_eq!(report.purged, 1);
                assert_eq!(
                    h.count("SELECT count(*) FROM tasks WHERE id=$1", uid(&t))
                        .await,
                    0
                );
                assert_eq!(
                    h.count("SELECT count(*) FROM activities WHERE id=$1", uid(&a))
                        .await,
                    0
                );
                assert_eq!(h.outbox(&t).await, 2);
            }
            other => panic!("non-serial outcome: {other:?}"),
        }
        assert_eq!(
            h.count("SELECT count(*) FROM entity_registry WHERE id=$1", uid(&t))
                .await,
            1
        );
        assert_eq!(
            h.count("SELECT count(*) FROM entity_links WHERE id=$1", uid(&link))
                .await,
            1
        );
    }
}
