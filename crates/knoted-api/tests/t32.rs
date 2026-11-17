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
        let table = match kind {
            "file" => "file_sources",
            "project" => "projects",
            "task" => "tasks",
            _ => panic!(),
        };
        sqlx::query(&if kind=="file" {"CREATE TABLE IF NOT EXISTS file_sources(id uuid PRIMARY KEY REFERENCES entity_registry(id),status text NOT NULL DEFAULT 'present',relative_path text NOT NULL DEFAULT '資料/report.md')".to_owned()} else {format!("CREATE TABLE IF NOT EXISTS {table}(id uuid PRIMARY KEY REFERENCES entity_registry(id),deleted_at timestamptz,archived_at timestamptz)")}).execute(&self.db.pool).await.unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,$2)")
            .bind(id)
            .bind(kind)
            .execute(&self.db.pool)
            .await
            .unwrap();
        sqlx::query(&format!("INSERT INTO {table}(id) VALUES($1)"))
            .bind(id)
            .execute(&self.db.pool)
            .await
            .unwrap();
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
