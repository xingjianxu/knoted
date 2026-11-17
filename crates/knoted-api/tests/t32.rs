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
    context::AppContext,
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
    async fn create(&self, title: &str, parent: Option<&str>) -> Value {
        let (s, h, b) = self
            .send(
                "POST",
                "/api/v1/notes",
                json!({"title":title,"parent_id":parent,"document":doc()}),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        assert_eq!(h["etag"], "\"1\"");
        assert_eq!(b["content_version"], 1);
        assert_eq!(b["document"], doc());
        b
    }
}
fn doc() -> Value {
    json!({"blocks":[]})
}

#[tokio::test]
async fn t32_3_tag_crud_and_entity_association_are_idempotent_and_undoable() {
    let h = Harness::new().await;
    let note = h.create("资料", None).await;
    let (s, _, tag) = h.send("POST", "/api/v1/tags", json!({"name":"研究"})).await;
    assert_eq!(s, StatusCode::CREATED, "{tag}");
    let endpoint = format!(
        "/api/v1/entities/note/{}/tags/{}",
        note["id"].as_str().unwrap(),
        tag["id"].as_str().unwrap()
    );
    for _ in 0..2 {
        assert_eq!(
            h.send("PUT", &endpoint, Value::Null).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let listing = format!(
        "/api/v1/entities/note/{}/tags",
        note["id"].as_str().unwrap()
    );
    assert_eq!(
        h.send("GET", &listing, Value::Null).await.2["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.send("DELETE", &endpoint, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("GET", &listing, Value::Null).await.2["items"],
        json!([])
    );
    let tagpath = format!("/api/v1/tags/{}", tag["id"].as_str().unwrap());
    assert_eq!(
        h.send(
            "PATCH",
            &tagpath,
            json!({"name":"更新","expected_version":1})
        )
        .await
        .2["version"],
        2
    );
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{tagpath}?expected_version=1"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{tagpath}?expected_version=2"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

impl Harness {
    async fn fixture(&self, kind: &str) -> Uuid {
        if kind == "project" {
            let (s, _, p) = self
                .send("POST", "/api/v1/projects", json!({"title":"fixture"}))
                .await;
            assert_eq!(s, StatusCode::CREATED, "{p}");
            return p["id"].as_str().unwrap().parse().unwrap();
        }
        let id = Uuid::new_v4();
        let mut tx = self.db.pool.begin().await.unwrap();
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,$2)")
            .bind(id)
            .bind(kind)
            .execute(&mut *tx)
            .await
            .unwrap();
        match kind {
            "task" => {
                sqlx::query("INSERT INTO tasks(id,status,created_at,updated_at) VALUES($1,'todo',now(),now())")
                    .bind(id).execute(&mut *tx).await.unwrap();
            }
            "file" => {
                sqlx::query("CREATE TABLE IF NOT EXISTS file_sources(id uuid PRIMARY KEY REFERENCES entity_registry(id),status text NOT NULL DEFAULT 'present',relative_path text NOT NULL DEFAULT '資料/report.md')").execute(&mut *tx).await.unwrap();
                sqlx::query("INSERT INTO file_sources(id) VALUES($1)")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            }
            _ => panic!(),
        }
        tx.commit().await.unwrap();
        id
    }
}
#[tokio::test]
async fn t32_1_stable_file_identity_never_points_to_replacement_and_can_be_removed() {
    let h = Harness::new().await;
    let note = h.create("note", None).await;
    let file = h.fixture("file").await;
    let input =
        json!({"source_type":"note","source_id":note["id"],"target_type":"file","target_id":file});
    let (s, _, link) = h.send("POST", "/api/v1/links", input.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    assert_eq!(
        h.send("POST", "/api/v1/links", input.clone()).await.2["id"],
        link["id"]
    );
    sqlx::query("UPDATE file_sources SET status='missing' WHERE id=$1")
        .bind(file)
        .execute(&h.db.pool)
        .await
        .unwrap();
    let replacement = h.fixture("file").await;
    assert_ne!(file, replacement);
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT relative_path FROM file_sources WHERE id IN ($1,$2)")
            .bind(file)
            .bind(replacement)
            .fetch_all(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(
        paths,
        vec!["資料/report.md".to_owned(), "資料/report.md".to_owned()]
    );
    let list = format!(
        "/api/v1/links?source_type=note&source_id={}",
        note["id"].as_str().unwrap()
    );
    assert_eq!(
        h.send("GET", &list, Value::Null).await.2["items"][0]["target_id"],
        file.to_string()
    );
    assert_eq!(
        h.send("POST", "/api/v1/links", input).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.send(
            "DELETE",
            &format!(
                "/api/v1/links/{}?expected_version=1",
                link["id"].as_str().unwrap()
            ),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("GET", &list, Value::Null).await.2["items"],
        json!([])
    );
}
#[tokio::test]
async fn t32_2_project_deletion_keeps_note_and_link_and_visibility_is_enforced() {
    let h = Harness::new().await;
    let note = h.create("note", None).await;
    let project = h.fixture("project").await;
    let task = h.fixture("task").await;
    for (kind, id) in [("project", project), ("task", task)] {
        let body =
            json!({"source_type":"note","source_id":note["id"],"target_type":kind,"target_id":id});
        assert_eq!(
            h.send("POST", "/api/v1/links", body).await.0,
            StatusCode::CREATED
        );
    }
    sqlx::query("DELETE FROM projects WHERE id=$1")
        .bind(project)
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/notes/{}", note["id"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let wrong = json!({"source_type":"note","source_id":note["id"],"target_type":"project","target_id":task});
    assert_eq!(
        h.send("POST", "/api/v1/links", wrong).await.0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE tasks SET archived_at=now() WHERE id=$1")
        .bind(task)
        .execute(&h.db.pool)
        .await
        .unwrap();
    let readonly =
        json!({"source_type":"task","source_id":task,"target_type":"note","target_id":note["id"]});
    assert_eq!(
        h.send("POST", "/api/v1/links", readonly).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn t32_missing_capabilities_deleted_endpoints_and_unregistered_ids_fail_closed() {
    let h = Harness::new().await;
    let note = h.create("visible", None).await;
    let registered = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(registered)
        .execute(&h.db.pool)
        .await
        .unwrap();
    for target in [registered, Uuid::new_v4()] {
        let input = json!({"source_type":"note","source_id":note["id"],"target_type":"file","target_id":target});
        assert_eq!(
            h.send("POST", "/api/v1/links", input).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/entities/file/{registered}/tags"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let id = Uuid::parse_str(note["id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE notes SET deleted_at=now() WHERE id=$1")
        .bind(id)
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/links?source_type=note&source_id={id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let input =
        json!({"source_type":"chat_message","source_id":id,"target_type":"note","target_id":id});
    assert_eq!(
        h.send("POST", "/api/v1/links", input).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn t32_tag_pagination_validation_visibility_and_typed_foreign_keys() {
    let h = Harness::new().await;
    for bad in ["".to_owned(), " ".to_owned(), "字".repeat(65)] {
        assert_eq!(
            h.send("POST", "/api/v1/tags", json!({"name":bad})).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut tags = Vec::new();
    for name in ["a", "b", "c"] {
        tags.push(h.send("POST", "/api/v1/tags", json!({"name":name})).await.2);
    }
    assert_eq!(
        h.send("POST", "/api/v1/tags", json!({"name":" a "}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let first = h.send("GET", "/api/v1/tags?limit=2", Value::Null).await.2;
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = h
        .send(
            "GET",
            &format!("/api/v1/tags?limit=2&cursor={cursor}"),
            Value::Null,
        )
        .await
        .2;
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    assert_ne!(first["items"][0]["id"], next["items"][0]["id"]);
    for query in ["limit=0", "limit=101", "cursor=broken"] {
        assert_eq!(
            h.send("GET", &format!("/api/v1/tags?{query}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let tag_id = Uuid::parse_str(tags[0]["id"].as_str().unwrap()).unwrap();
    for kind in ["file", "project", "task"] {
        let entity = h.fixture(kind).await;
        let endpoint = format!("/api/v1/entities/{kind}/{entity}/tags/{tag_id}");
        assert_eq!(
            h.send("PUT", &endpoint, Value::Null).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            h.send(
                "GET",
                &format!("/api/v1/entities/{kind}/{entity}/tags?cursor={cursor}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        if kind != "file" {
            let table = if kind == "project" {
                "projects"
            } else {
                "tasks"
            };
            sqlx::query(&format!("UPDATE {table} SET archived_at=now() WHERE id=$1"))
                .bind(entity)
                .execute(&h.db.pool)
                .await
                .unwrap();
            assert_eq!(
                h.send(
                    "GET",
                    &format!("/api/v1/entities/{kind}/{entity}/tags"),
                    Value::Null
                )
                .await
                .0,
                StatusCode::OK
            );
            assert_eq!(
                h.send("DELETE", &endpoint, Value::Null).await.0,
                StatusCode::CONFLICT
            );
        }
        let wrong = sqlx::query("INSERT INTO project_tags(entity_id,tag_id) VALUES($1,$2)")
            .bind(entity)
            .bind(tag_id)
            .execute(&h.db.pool)
            .await;
        if kind != "project" {
            assert!(wrong.is_err(), "wrong registry type must fail the FK");
        }
    }
}

#[tokio::test]
async fn t32_links_require_auth_csrf_and_cas_and_paginate_without_rebinding() {
    let h = Harness::new().await;
    let note = h.create("note", None).await;
    let source = note["id"].as_str().unwrap();
    for _ in 0..3 {
        let other = h.create("target", None).await;
        let body = json!({"source_type":"note","source_id":source,"target_type":"note","target_id":other["id"]});
        let (a, b) = tokio::join!(
            h.send("POST", "/api/v1/links", body.clone()),
            h.send("POST", "/api/v1/links", body)
        );
        assert_eq!(a.0, StatusCode::CREATED);
        assert_eq!(a.2["id"], b.2["id"]);
    }
    let list = format!("/api/v1/links?source_type=note&source_id={source}&limit=2");
    let first = h.send("GET", &list, Value::Null).await.2;
    let next = h
        .send(
            "GET",
            &format!("{list}&cursor={}", first["next_cursor"].as_str().unwrap()),
            Value::Null,
        )
        .await
        .2;
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    let link = first["items"][0]["id"].as_str().unwrap();
    assert_eq!(
        h.send(
            "DELETE",
            &format!("/api/v1/links/{link}?expected_version=9"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    for (cookie, csrf, expected) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, false, StatusCode::FORBIDDEN),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/v1/links")
            .header("content-type", "application/json")
            .header("origin", AuthConfig::default().origin);
        if cookie {
            req = req.header("cookie", &h.cookie)
        }
        if csrf {
            req = req.header("x-csrf-token", &h.csrf)
        }
        let r = h
            .app
            .clone()
            .oneshot(req.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), expected);
    }
}

#[tokio::test]
async fn t32_cas_conflicts_include_current_version_for_each_versioned_operation() {
    let h = Harness::new().await;
    let tag = h
        .send("POST", "/api/v1/tags", json!({"name":"CAS"}))
        .await
        .2;
    let tag_path = format!("/api/v1/tags/{}", tag["id"].as_str().unwrap());
    assert_eq!(
        h.send(
            "PATCH",
            &tag_path,
            json!({"name":"updated","expected_version":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    for (method, path, body) in [
        (
            "PATCH",
            tag_path.clone(),
            json!({"name":"stale","expected_version":1}),
        ),
        (
            "DELETE",
            format!("{tag_path}?expected_version=1"),
            Value::Null,
        ),
    ] {
        let (status, _, error) = h.send(method, &path, body).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "version_conflict");
        assert_eq!(
            error["current_version"], 2,
            "{method} must return the current tag version: {error}"
        );
    }
    let a = h.create("source", None).await;
    let b = h.create("target", None).await;
    let link = h.send("POST", "/api/v1/links", json!({"source_type":"note","source_id":a["id"],"target_type":"note","target_id":b["id"]})).await.2;
    let (status, _, error) = h
        .send(
            "DELETE",
            &format!(
                "/api/v1/links/{}?expected_version=9",
                link["id"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "version_conflict");
    assert_eq!(error["current_version"], 1, "{error}");
}

// The gate holds A until link(A -> B) has queued before move(B -> A).
// Before the fix, the move holds B and queues on A; releasing the gate makes
// link hold A while waiting on B, producing a real PostgreSQL deadlock.
// With the shared tree protocol, move waits on link's tree lock before B.
#[tokio::test]
async fn t32_note_link_and_tree_move_share_lock_order_without_deadlock() {
    use sqlx::Connection;
    let h = Harness::new().await;
    let mut notes = [h.create("a", None).await, h.create("b", None).await];
    notes.sort_by_key(|n| Uuid::parse_str(n["id"].as_str().unwrap()).unwrap());
    let a = notes[0]["id"].as_str().unwrap();
    let b = notes[1]["id"].as_str().unwrap();
    let mut gate = sqlx::PgConnection::connect(&knoted_core::db::testing::test_database_url())
        .await
        .unwrap();
    sqlx::query(&format!("SET search_path TO {}, public", h.db.schema()))
        .execute(&mut gate)
        .await
        .unwrap();
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await
        .unwrap();
    let mut gate_tx = gate.begin().await.unwrap();
    sqlx::query("SELECT id FROM notes WHERE id=$1 FOR UPDATE")
        .bind(Uuid::parse_str(a).unwrap())
        .execute(&mut *gate_tx)
        .await
        .unwrap();
    let request = |path: &str, body: Value| {
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
        "/api/v1/links",
        json!({"source_type":"note","source_id":a,"target_type":"note","target_id":b}),
    )));
    let link_pid = blocked_backend(&mut gate_tx, &[gate_pid], None).await;
    if link_pid.is_none() {
        gate_tx.rollback().await.unwrap();
        let _ = link.await;
        panic!("link did not reach the controlled note-row gate");
    }
    let link_pid = link_pid.unwrap();
    let moved = tokio::spawn(h.app.clone().oneshot(request(
        &format!("/api/v1/notes/{b}/move"),
        json!({"parent_id":a,"position":0,"expected_version":1}),
    )));
    let move_pid = blocked_backend(&mut gate_tx, &[gate_pid, link_pid], Some(link_pid)).await;
    gate_tx.commit().await.unwrap();
    let (link_response, move_response) =
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            (link.await.unwrap().unwrap(), moved.await.unwrap().unwrap())
        })
        .await
        .expect("both requests must finish after releasing the gate");
    assert!(
        move_pid.is_some(),
        "move must enter the controlled lock queue before the gate opens"
    );
    let link_status = link_response.status();
    let move_status = move_response.status();
    assert_eq!(
        link_status,
        StatusCode::CREATED,
        "link returned {link_status}; move returned {move_status}"
    );
    assert_eq!(move_status, StatusCode::OK);
    let moved = h
        .send("GET", &format!("/api/v1/notes/{b}"), Value::Null)
        .await
        .2;
    assert_eq!(moved["parent_id"], a);
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
