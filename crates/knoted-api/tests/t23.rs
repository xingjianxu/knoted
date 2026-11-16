//! Real HTTP and PostgreSQL checks for the note persistence boundary.
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
    async fn events(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM outbox")
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
}
fn doc() -> Value {
    json!({"blocks":[{"id":"p1","type":"paragraph","content":[{"type":"text","text":"中文原文"}]}]})
}
fn path(n: &Value) -> String {
    format!("/api/v1/notes/{}", n["id"].as_str().unwrap())
}

// Removing the SQL version predicate must allow two winners and fail this test.
#[tokio::test]
async fn t23_1_concurrent_updates_have_exactly_one_winner() {
    let h = Harness::new().await;
    let n = h.create("initial", None).await;
    let p = path(&n);
    let (a, b) = tokio::join!(
        h.send("PATCH", &p, json!({"expected_version":1,"title":"a"})),
        h.send("PATCH", &p, json!({"expected_version":1,"title":"b"}))
    );
    let statuses = [a.0, b.0];
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    let loser = if a.0 == StatusCode::CONFLICT { a } else { b };
    assert_eq!(loser.0, StatusCode::CONFLICT);
    assert_eq!(loser.2["current_version"], 2);
    let (s, headers, n) = h.send("GET", &p, Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(n["content_version"], 2);
    assert_eq!(headers["etag"], "\"2\"");
    assert_eq!(h.events().await, 2);
    let event: Value =
        sqlx::query_scalar("SELECT payload FROM outbox ORDER BY created_at DESC LIMIT 1")
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(event["content_version"], 2);
    assert_eq!(event["source_id"], n["id"]);
}

// Removing deleted_at checks would expose deleted content or allow stale writes.
#[tokio::test]
async fn t23_2_deleted_note_is_hidden_from_reads_and_writes() {
    let h = Harness::new().await;
    let n = h.create("gone", None).await;
    let p = path(&n);
    assert_eq!(
        h.send("DELETE", &format!("{p}?expected_version=1"), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send("GET", &p, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.send("PATCH", &p, json!({"expected_version":2,"title":"back"}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.send("GET", "/api/v1/notes", Value::Null).await.2["items"],
        json!([])
    );
    let row: (i64, bool) =
        sqlx::query_as("SELECT content_version,deleted_at IS NOT NULL FROM notes WHERE id=$1")
            .bind(Uuid::parse_str(n["id"].as_str().unwrap()).unwrap())
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_eq!(row, (2, true));
    assert_eq!(h.events().await, 2);
}

// A real database fault after the content write must roll back all side effects.
#[tokio::test]
async fn t23_3_failed_transaction_has_no_outbox_or_partial_note() {
    let h = Harness::new().await;
    let n = h.create("original", None).await;
    sqlx::raw_sql("CREATE FUNCTION reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected event failure'; END $$; CREATE TRIGGER reject_event BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_event();").execute(&h.db.pool).await.unwrap();
    assert_eq!(
        h.send(
            "PATCH",
            &path(&n),
            json!({"expected_version":1,"title":"lost"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        h.send(
            "POST",
            "/api/v1/notes",
            json!({"title":"lost create","document":doc()})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        h.send("GET", &path(&n), Value::Null).await.2["title"],
        "original"
    );
    assert_eq!(h.events().await, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM entity_registry WHERE entity_type='note'"
        )
        .fetch_one(&h.db.pool)
        .await
        .unwrap(),
        1
    );
    sqlx::raw_sql("DROP TRIGGER reject_event ON outbox")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.send(
            "PATCH",
            &path(&n),
            json!({"expected_version":1,"title":"retry"})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn t23_crud_preserves_content_and_noop_version() {
    let h = Harness::new().await;
    let n = h.create("标题", None).await;
    let (s, headers, b) = h.send("GET", &path(&n), Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(headers["etag"], "\"1\"");
    assert_eq!(b, n);
    let (s, _, b) = h
        .send(
            "PATCH",
            &path(&n),
            json!({"expected_version":1,"title":"标题","document":doc()}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["content_version"], 1);
    assert_eq!(h.events().await, 1);
    let changed = json!({"blocks":[]});
    let (s, _, b) = h
        .send(
            "PATCH",
            &path(&n),
            json!({"expected_version":1,"document":changed}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["content_version"], 2);
    assert_eq!(b["document"], changed);
    assert_eq!(b["title"], "标题");
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{}?expected_version=1", path(&n)),
            Value::Null
        )
        .await
        .2["current_version"],
        2
    );
    assert_eq!(
        h.send(
            "GET",
            &format!("/api/v1/notes/{}", Uuid::new_v4()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn t23_parent_visibility_order_and_delete_guard() {
    let h = Harness::new().await;
    let parent = h.create("parent", None).await;
    let id = parent["id"].as_str().unwrap();
    let a = h.create("a", Some(id)).await;
    let b = h.create("b", Some(id)).await;
    assert_eq!(a["position"], 0);
    assert_eq!(b["position"], 1);
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{}?expected_version=1", path(&parent)),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{}?expected_version=1", path(&a)),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(h.send("GET", &path(&b), Value::Null).await.2["position"], 0);
    let items = h
        .send("GET", &format!("/api/v1/notes?parent_id={id}"), Value::Null)
        .await
        .2;
    assert_eq!(items["items"].as_array().unwrap().len(), 1);
    let bad = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"bad","parent_id":a["id"],"document":doc()}),
        )
        .await;
    assert_eq!(bad.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn t23_pagination_binds_filter_and_checks_cursor() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    h.create("b", None).await;
    h.create("c", None).await;
    let (s, _, page) = h.send("GET", "/api/v1/notes?limit=2", Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = h
        .send(
            "GET",
            &format!("/api/v1/notes?limit=2&cursor={cursor}"),
            Value::Null,
        )
        .await
        .2;
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    assert!(next["next_cursor"].is_null());
    assert_ne!(next["items"][0]["id"], page["items"][1]["id"]);
    for q in [
        "limit=0".to_string(),
        "limit=101".to_string(),
        "cursor=%%%".to_string(),
        format!("parent_id={}&cursor={cursor}", a["id"].as_str().unwrap()),
        format!(
            "cursor={}",
            URL_SAFE_NO_PAD.encode(br#"{"v":1,"position":-1,"id":"bad","parent_id":null}"#)
        ),
    ] {
        assert_eq!(
            h.send("GET", &format!("/api/v1/notes?{q}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn t23_validation_rejects_unknown_null_and_invalid_document() {
    let h = Harness::new().await;
    let n = h.create("valid", None).await;
    for input in [
        json!({"expected_version":1}),
        json!({"expected_version":0,"title":"bad"}),
        json!({"expected_version":1,"title":null}),
        json!({"expected_version":1,"document":null}),
        json!({"expected_version":1,"content_version":99,"title":"bad"}),
        json!({"expected_version":1,"title":"中".repeat(501)}),
        json!({"expected_version":1,"document":{"blocks":[{"id":"x","type":"html"}]}}),
    ] {
        let (s, _, b) = h.send("PATCH", &path(&n), input).await;
        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY].contains(&s),
            "{s} {b}"
        );
        assert!(b["request_id"].is_string());
    }
    assert_eq!(h.events().await, 1);
    let (s, _, _) = h
        .send(
            "POST",
            "/api/v1/notes",
            json!({"title":"中".repeat(500),"document":doc()}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let huge = json!({"blocks":[{"id":"x","type":"paragraph","content":[{"type":"text","text":"x".repeat(2*1024*1024)}]}]});
    let (s, _, b) = h
        .send(
            "PATCH",
            &path(&n),
            json!({"expected_version":1,"document":huge}),
        )
        .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(b["request_id"].is_string());
}

#[tokio::test]
async fn t23_auth_and_csrf_cover_note_routes() {
    let h = Harness::new().await;
    for method in ["GET", "POST", "PATCH", "DELETE"] {
        let p = if method == "GET" || method == "POST" {
            "/api/v1/notes".to_string()
        } else {
            format!("/api/v1/notes/{}", Uuid::new_v4())
        };
        let r = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(p)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    let r = h
        .app
        .clone()
        .oneshot(
            Request::post("/api/v1/notes")
                .header("cookie", &h.cookie)
                .header("origin", "https://evil.example")
                .header("x-csrf-token", &h.csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn t23_parent_delete_racing_child_creation_stays_consistent() {
    let h = Harness::new().await;
    let parent = h.create("p", None).await;
    let p = format!("{}?expected_version=1", path(&parent));
    let (del, create) = tokio::join!(
        h.send("DELETE", &p, Value::Null),
        h.send(
            "POST",
            "/api/v1/notes",
            json!({"title":"c","parent_id":parent["id"],"document":doc()})
        )
    );
    assert!(
        (del.0 == StatusCode::NO_CONTENT && create.0 == StatusCode::CONFLICT)
            || (del.0 == StatusCode::CONFLICT && create.0 == StatusCode::CREATED)
    );
    let broken:i64=sqlx::query_scalar("SELECT count(*) FROM notes c JOIN notes p ON c.parent_id=p.id WHERE c.deleted_at IS NULL AND p.deleted_at IS NOT NULL").fetch_one(&h.db.pool).await.unwrap();
    assert_eq!(broken, 0);
}

#[tokio::test]
async fn t23_cursor_rejects_missing_filter_and_wrong_types() {
    let h = Harness::new().await;
    for raw in [
        json!({"v":1,"position":0,"id":Uuid::new_v4()}),
        json!({"v":2,"position":0,"id":Uuid::new_v4(),"parent_id":null}),
        json!({"v":1,"position":"0","id":Uuid::new_v4(),"parent_id":null}),
        json!({"v":1,"position":0,"id":Uuid::new_v4(),"parent_id":null,"extra":true}),
    ] {
        let cursor = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&raw).unwrap());
        assert_eq!(
            h.send(
                "GET",
                &format!("/api/v1/notes?cursor={cursor}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn t23_raw_document_preserves_duplicate_key_rejection() {
    let h = Harness::new().await;
    for raw in [
        r#"{"title":"x","document":{"blocks":[],"blocks":[]}}"#,
        r#"{"title":"x","title":"y","document":{"blocks":[]}}"#,
        r#"{"title":"x","document":{"blocks":[{"id":"x","type":"paragraph","content":[{"type":"text","text":"x","text":"y"}]}]}}"#,
    ] {
        let r = h
            .app
            .clone()
            .oneshot(
                Request::post("/api/v1/notes")
                    .header("cookie", &h.cookie)
                    .header("origin", AuthConfig::default().origin)
                    .header("x-csrf-token", &h.csrf)
                    .header("content-type", "application/json")
                    .body(Body::from(raw))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!([StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY].contains(&r.status()));
    }
    assert_eq!(h.events().await, 0);
}

#[tokio::test]
async fn t23_document_exact_size_boundary_is_enforced() {
    let h = Harness::new().await;
    // This literal format is a valid paragraph; only the text padding changes.
    let prefix = r#"{"blocks":[{"id":"x","type":"paragraph","content":[{"type":"text","text":""#;
    let suffix = r#""}]}]}"#;
    for (bytes, want) in [
        (2 * 1024 * 1024, StatusCode::CREATED),
        (2 * 1024 * 1024 + 1, StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let raw = format!(
            "{prefix}{}{suffix}",
            "x".repeat(bytes - prefix.len() - suffix.len())
        );
        assert_eq!(raw.len(), bytes);
        let r = h
            .app
            .clone()
            .oneshot(
                Request::post("/api/v1/notes")
                    .header("cookie", &h.cookie)
                    .header("origin", AuthConfig::default().origin)
                    .header("x-csrf-token", &h.csrf)
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"title":"boundary","document":{raw}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), want);
    }
    assert_eq!(h.events().await, 1);
}

#[tokio::test]
async fn t23_concurrent_root_appends_assign_distinct_positions() {
    let h = Harness::new().await;
    let (a, b) = tokio::join!(h.create("a", None), h.create("b", None));
    assert!(
        (a["position"] == 0 && b["position"] == 1) || (a["position"] == 1 && b["position"] == 0)
    );
}

#[tokio::test]
async fn t23_syntax_errors_use_request_id_envelope() {
    let h = Harness::new().await;
    let n = h.create("a", None).await;
    for (method, p) in [
        ("GET", "/api/v1/notes/not-a-uuid".to_string()),
        ("DELETE", path(&n)),
        ("DELETE", format!("{}?expected_version=no", path(&n))),
        ("GET", "/api/v1/notes?parent_id=null".to_string()),
    ] {
        let (s, headers, b) = h.send(method, &p, Value::Null).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(
            b["request_id"].as_str().unwrap(),
            headers["x-request-id"].to_str().unwrap()
        );
    }
}

// PostgreSQL text/jsonb cannot persist U+0000: reject this permanently invalid
// content as a field error, rather than claiming a transient dependency fault.
#[tokio::test]
async fn t23_nul_content_is_422_and_has_no_write_side_effects() {
    let h = Harness::new().await;
    let n = h.create("original", None).await;
    let nul_text = json!({"blocks":[{"id":"x","type":"paragraph","content":[{"type":"text","text":"a\u{0}b"}]}]});
    let nul_id = json!({"blocks":[{"id":"a\u{0}b","type":"paragraph"}]});
    let creates = [
        json!({"title":"a\u{0}b","document":doc()}),
        json!({"title":"bad text","document":nul_text}),
        json!({"title":"bad id","document":nul_id}),
    ];
    for input in creates {
        let (s, _, body) = h.send("POST", "/api/v1/notes", input).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], "validation_failed");
    }
    let updates = [
        json!({"expected_version":1,"title":"a\u{0}b"}),
        json!({"expected_version":1,"document":nul_text}),
        json!({"expected_version":1,"document":nul_id}),
    ];
    for input in updates {
        assert_eq!(
            h.send("PATCH", &path(&n), input).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let got = h.send("GET", &path(&n), Value::Null).await.2;
    assert_eq!(got["title"], "original");
    assert_eq!(got["content_version"], 1);
    assert_eq!(got["document"], doc());
    assert_eq!(h.events().await, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM entity_registry WHERE entity_type='note'"
        )
        .fetch_one(&h.db.pool)
        .await
        .unwrap(),
        1
    );
}
