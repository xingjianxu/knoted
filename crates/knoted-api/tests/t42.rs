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

struct Harness {
    db: TestDb,
    app: Router,
    cookie: String,
    csrf: String,
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
}

#[tokio::test]
async fn t42_trash_is_authenticated_paginated_filtered_and_restore_uses_domain() {
    let h = Harness::new().await;
    let (s, _, empty) = h.send("GET", "/api/v1/trash", Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{empty}");
    assert_eq!(empty, json!({"items":[],"next_cursor":null}));
    let (_, _, note) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"deleted note","document":{"blocks":[]}}),
        )
        .await;
    let id = note["id"].as_str().unwrap();
    let (s, _, _) = h
        .send(
            "DELETE",
            &format!("/api/v1/notes/{id}?expected_version=1"),
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, page) = h
        .send("GET", "/api/v1/trash?entity_type=note&limit=1", Value::Null)
        .await;
    assert_eq!(page["items"][0]["id"], id);
    assert_eq!(page["items"][0]["version"], 2);
    assert_eq!(page["items"][0]["can_restore"], true);
    assert!(page["items"][0]["reason"].is_null());
    let (s, _, _) = h
        .send(
            "POST",
            &format!("/api/v1/notes/{id}/restore"),
            json!({"expected_version":2}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    for query in [
        "limit=0",
        "limit=101",
        "entity_type=file",
        "cursor=invalid",
        "extra=1",
    ] {
        let (s, _, _) = h
            .send("GET", &format!("/api/v1/trash?{query}"), Value::Null)
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{query}");
    }
    let r = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/trash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    h.db.drop().await;
}

#[tokio::test]
async fn t42_1_parent_missing_and_2_archived_registry_only_excluded() {
    let h = Harness::new().await;
    let ctx = AppContext::new(h.db.pool.clone());
    use knoted_core::notes::service::{self, CreateNote};
    let parent = service::create(
        &ctx,
        CreateNote {
            title: "parent".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let child = service::create(
        &ctx,
        CreateNote {
            title: "child".into(),
            parent_id: Some(parent.id),
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE notes SET deleted_at=now() - interval '40 days' WHERE id=$1 OR id=$2")
        .bind(parent.id)
        .bind(child.id)
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (_, _, task) = h
        .send("POST", "/api/v1/tasks", json!({"title":"archived"}))
        .await;
    let id = task["id"].as_str().unwrap();
    let (s, _, _) = h
        .send(
            "POST",
            &format!("/api/v1/tasks/{id}/archive"),
            json!({"expected_version":1}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, page) = h.send("GET", "/api/v1/trash", Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let c = items
        .iter()
        .find(|x| x["id"] == child.id.to_string())
        .unwrap();
    assert_eq!(c["can_restore"], false);
    assert!(c["reason"].as_str().unwrap().contains("父"));
    let p = items
        .iter()
        .find(|x| x["id"] == parent.id.to_string())
        .unwrap();
    assert_eq!(p["can_restore"], true, "40 days is not a restore deadline");
    let (s, _, _) = h
        .send(
            "POST",
            &format!("/api/v1/notes/{}/restore", child.id),
            json!({"expected_version":1,"restore_parent":{"parent_id":null}}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    h.db.drop().await;
}

#[tokio::test]
async fn t42_http_errors_never_expose_database_details() {
    let h = Harness::new().await;
    sqlx::query("ALTER TABLE notes RENAME TO private_trash_notes")
        .execute(&h.db.pool)
        .await
        .unwrap();
    let (status, _, body) = h.send("GET", "/api/v1/trash", Value::Null).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "dependency_unavailable");
    assert!(!body.to_string().contains("private_trash_notes"));
    h.db.drop().await;
}
