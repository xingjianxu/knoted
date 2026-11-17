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

impl Harness {
    async fn move_note(
        &self,
        n: &Value,
        parent: Value,
        position: i64,
        version: i64,
    ) -> (StatusCode, HeaderMap, Value) {
        self.send(
            "POST",
            &format!("{}/move", path(n)),
            json!({"parent_id":parent,"position":position,"expected_version":version}),
        )
        .await
    }
    async fn delete(&self, n: &Value, version: i64) {
        assert_eq!(
            self.send(
                "DELETE",
                &format!("{}?expected_version={version}", path(n)),
                Value::Null
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    async fn restore(&self, n: &Value, body: Value) -> (StatusCode, HeaderMap, Value) {
        self.send("POST", &format!("{}/restore", path(n)), body)
            .await
    }
    async fn state(&self) -> Value {
        sqlx::query_scalar(
            "SELECT COALESCE(jsonb_agg(to_jsonb(n) ORDER BY id),'[]'::jsonb) FROM notes n",
        )
        .fetch_one(&self.db.pool)
        .await
        .unwrap()
    }
    async fn order(&self, parent: Option<&Value>) -> Vec<String> {
        let parent = parent.map(|v| Uuid::parse_str(v["id"].as_str().unwrap()).unwrap());
        let rows: Vec<(String,i64)>=sqlx::query_as("SELECT title,position FROM notes WHERE parent_id IS NOT DISTINCT FROM $1 AND deleted_at IS NULL ORDER BY position,id").bind(parent).fetch_all(&self.db.pool).await.unwrap();
        for (i, (_, position)) in rows.iter().enumerate() {
            assert_eq!(*position, i as i64);
        }
        rows.into_iter().map(|(title, _)| title).collect()
    }
}

// Omitting the ancestor check would make a valid UUID parent create a cycle.
#[tokio::test]
async fn t24_1_move_into_descendant_or_self_is_atomic_conflict() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", a["id"].as_str()).await;
    let c = h.create("c", b["id"].as_str()).await;
    let before = h.state().await;
    for parent in [&a, &b, &c] {
        let r = h.move_note(&a, parent["id"].clone(), 0, 1).await;
        assert_eq!(r.0, StatusCode::CONFLICT, "{}", r.2);
        assert_eq!(r.2["current_version"], 1);
        assert!(r.2["request_id"].is_string());
        assert_eq!(h.state().await, before);
    }
    assert_eq!(h.events().await, 3);
}

// This is an existing T23 guard: deletion must never cascade into children.
#[tokio::test]
async fn t24_2_delete_never_implicitly_cascades() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", a["id"].as_str()).await;
    let before = h.state().await;
    assert_eq!(
        h.send(
            "DELETE",
            &format!("{}?expected_version=1", path(&a)),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, 2);
    h.delete(&b, 1).await;
    h.delete(&a, 1).await;
    assert_eq!(h.order(None).await, Vec::<String>::new());
}

// Incorrect shifts would duplicate positions, reorder bystanders or leave holes.
#[tokio::test]
async fn t24_3_sibling_order_is_stable_across_moves_and_restore() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let c = h.create("c", None).await;
    let parent = h.create("parent", None).await;
    let d = h.create("d", parent["id"].as_str()).await;
    let r = h.move_note(&c, Value::Null, 0, 1).await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.1["etag"], "\"2\"");
    assert_eq!(h.order(None).await, ["c", "a", "b", "parent"]);
    assert_eq!(h.move_note(&c, Value::Null, 2, 2).await.0, StatusCode::OK);
    assert_eq!(h.order(None).await, ["a", "b", "c", "parent"]);
    assert_eq!(
        h.move_note(&b, parent["id"].clone(), 0, 1).await.0,
        StatusCode::OK
    );
    assert_eq!(h.order(None).await, ["a", "c", "parent"]);
    assert_eq!(h.order(Some(&parent)).await, ["b", "d"]);
    assert_eq!(
        h.send("GET", &path(&d), Value::Null).await.2["content_version"],
        1
    );
    h.delete(&b, 2).await;
    assert_eq!(h.order(Some(&parent)).await, ["d"]);
    let r = h.restore(&b, json!({"expected_version":3})).await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.2["content_version"], 4);
    assert_eq!(h.order(Some(&parent)).await, ["b", "d"]);
    assert_eq!(
        h.send("GET", &path(&a), Value::Null).await.2["content_version"],
        1
    );
    let before = h.state().await;
    let events = h.events().await;
    assert_eq!(
        h.move_note(&b, parent["id"].clone(), 0, 4).await.2["content_version"],
        4
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, events);
}

// Flattening absent and nullable fields would silently lose the original parent.
#[tokio::test]
async fn t24_restore_preserves_parent_unless_explicitly_overridden() {
    let h = Harness::new().await;
    let p = h.create("p", None).await;
    let n = h.create("n", p["id"].as_str()).await;
    h.delete(&n, 1).await;
    assert_eq!(
        h.restore(&n, json!({"expected_version":2})).await.2["parent_id"],
        p["id"]
    );
    h.delete(&n, 3).await;
    h.delete(&p, 1).await;
    let before = h.state().await;
    let events = h.events().await;
    assert_eq!(
        h.restore(&n, json!({"expected_version":4})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, events);
    let r = h
        .restore(
            &n,
            json!({"expected_version":4,"restore_parent":{"parent_id":null}}),
        )
        .await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.1["etag"], "\"5\"");
    assert!(r.2["parent_id"].is_null());
    assert!(r.2["deleted_at"].is_null());
    assert_eq!(r.2["document"], doc());
    assert_eq!(h.order(None).await, ["n"]);
}

// Stale versions, deleted targets and invalid parents must not shift any sibling.
#[tokio::test]
async fn t24_rejections_leave_versions_positions_and_events_unchanged() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    h.delete(&b, 1).await;
    let before = h.state().await;
    let events = h.events().await;
    for parent in [b["id"].clone(), json!(Uuid::new_v4())] {
        assert_eq!(
            h.move_note(&a, parent.clone(), 0, 1).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            h.restore(
                &b,
                json!({"expected_version":2,"restore_parent":{"parent_id":parent}})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        h.move_note(&a, Value::Null, 0, 2).await.2["current_version"],
        1
    );
    assert_eq!(
        h.restore(&b, json!({"expected_version":1})).await.2["current_version"],
        2
    );
    assert_eq!(
        h.restore(&a, json!({"expected_version":1})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.move_note(&b, Value::Null, 0, 2).await.0,
        StatusCode::NOT_FOUND
    );
    let missing = json!({"id":Uuid::new_v4()});
    assert_eq!(
        h.restore(&missing, json!({"expected_version":1})).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.move_note(&missing, Value::Null, 0, 1).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, events);
}

// Restoring under a descendant must traverse even deleted intermediate nodes.
#[tokio::test]
async fn t24_restore_cannot_cycle_through_deleted_ancestor() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", a["id"].as_str()).await;
    let c = h.create("c", b["id"].as_str()).await;
    h.delete(&c, 1).await;
    h.delete(&b, 1).await;
    h.delete(&a, 1).await;
    assert_eq!(
        h.restore(
            &c,
            json!({"expected_version":2,"restore_parent":{"parent_id":a["id"]}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.restore(&a, json!({"expected_version":2})).await.0,
        StatusCode::OK
    );
    // b is deleted but still lies between c and a in the stored ancestry.
    let before = h.state().await;
    assert_eq!(
        h.move_note(&a, b["id"].clone(), 0, 3).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.restore(
            &b,
            json!({"expected_version":2,"restore_parent":{"parent_id":b["id"]}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.state().await, before);
}

// Failing the outbox write after shifts must roll back the entire structural edit.
#[tokio::test]
async fn t24_event_failure_rolls_back_moves_and_restores() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let c = h.create("c", None).await;
    h.delete(&c, 1).await;
    let before = h.state().await;
    let events = h.events().await;
    sqlx::raw_sql("CREATE FUNCTION reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected event failure'; END $$; CREATE TRIGGER reject_event BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_event();").execute(&h.db.pool).await.unwrap();
    assert_eq!(
        h.move_note(&b, a["id"].clone(), 0, 1).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        h.restore(&c, json!({"expected_version":2})).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, events);
    sqlx::raw_sql("DROP TRIGGER reject_event ON outbox")
        .execute(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        h.restore(&c, json!({"expected_version":2})).await.0,
        StatusCode::OK
    );
}

// Without shared tree serialization both opposite moves can pass ancestor checks.
#[tokio::test]
async fn t24_concurrent_moves_cannot_create_a_cycle() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let (ra, rb) = tokio::join!(
        h.move_note(&a, b["id"].clone(), 0, 1),
        h.move_note(&b, a["id"].clone(), 0, 1)
    );
    assert!([ra.0, rb.0].contains(&StatusCode::OK));
    assert!([ra.0, rb.0].contains(&StatusCode::CONFLICT));
    assert_eq!(h.events().await, 3);
}

// Shared CAS and tree locks must permit exactly one winner across lifecycle races.
#[tokio::test]
async fn t24_concurrent_writes_have_one_winner_and_contiguous_siblings() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let bp = path(&b);
    let (ra, rb) = tokio::join!(
        h.move_note(&b, Value::Null, 0, 1),
        h.send("PATCH", &bp, json!({"expected_version":1,"title":"new"}))
    );
    assert_eq!(
        [ra.0, rb.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [ra.0, rb.0]
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    h.delete(&a, 1).await;
    let (ra, rb) = tokio::join!(
        h.restore(&a, json!({"expected_version":2})),
        h.restore(&a, json!({"expected_version":2}))
    );
    assert_eq!(
        [ra.0, rb.0]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [ra.0, rb.0]
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_eq!(h.order(None).await.len(), 2);
    assert_eq!(h.events().await, 5);
}

// Treating missing parent_id as null or accepting null restore_parent breaks T03.
#[tokio::test]
async fn t24_request_validation_and_auth_cover_actions() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    for (action, body) in [
        ("move", json!({"expected_version":1,"position":0})),
        (
            "move",
            json!({"expected_version":1,"parent_id":null,"position":-1}),
        ),
        (
            "move",
            json!({"expected_version":0,"parent_id":null,"position":0}),
        ),
        (
            "restore",
            json!({"expected_version":1,"restore_parent":null}),
        ),
        ("restore", json!({"expected_version":1,"restore_parent":{}})),
        (
            "restore",
            json!({"expected_version":1,"restore_parent":{"parent_id":null,"extra":1}}),
        ),
        ("restore", json!({"expected_version":0})),
    ] {
        let r = h
            .send("POST", &format!("{}/{action}", path(&a)), body)
            .await;
        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY].contains(&r.0),
            "{} {}",
            r.0,
            r.2
        );
    }
    for action in ["move", "restore"] {
        let p = format!("{}/{action}", path(&a));
        let r = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&p)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&p)
                    .header("cookie", &h.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(h.events().await, 1);
}

// Racing inserts at the same index must retain both notes and stable bystanders.
#[tokio::test]
async fn t24_concurrent_moves_to_same_parent_keep_contiguous_order() {
    let h = Harness::new().await;
    let p = h.create("p", None).await;
    h.create("first", p["id"].as_str()).await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let (ra, rb) = tokio::join!(
        h.move_note(&a, p["id"].clone(), 0, 1),
        h.move_note(&b, p["id"].clone(), 0, 1)
    );
    assert_eq!(ra.0, StatusCode::OK);
    assert_eq!(rb.0, StatusCode::OK);
    let order = h.order(Some(&p)).await;
    assert_eq!(order.len(), 3);
    assert_eq!(order[2], "first");
    assert!(order[..2].contains(&"a".to_string()) && order[..2].contains(&"b".to_string()));
    assert_eq!(h.order(None).await, ["p"]);
}

// Deleting a destination while a move enters it must not strand a live child.
#[tokio::test]
async fn t24_move_and_delete_parent_serialize_without_orphans() {
    let h = Harness::new().await;
    let p = h.create("p", None).await;
    let a = h.create("a", None).await;
    let delete_path = format!("{}?expected_version=1", path(&p));
    let (m, d) = tokio::join!(
        h.move_note(&a, p["id"].clone(), 0, 1),
        h.send("DELETE", &delete_path, Value::Null)
    );
    assert!(matches!(
        (m.0, d.0),
        (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::NO_CONTENT)
    ));
    let orphans:i64=sqlx::query_scalar("SELECT count(*) FROM notes n JOIN notes p ON n.parent_id=p.id WHERE n.deleted_at IS NULL AND p.deleted_at IS NOT NULL").fetch_one(&h.db.pool).await.unwrap();
    assert_eq!(orphans, 0);
    h.order(None).await;
    h.order(Some(&p)).await;
    assert_eq!(h.events().await, 3);
}

// A huge index cannot create gaps; a new restore parent must be used verbatim.
#[tokio::test]
async fn t24_move_clamps_to_end_and_restore_accepts_new_live_parent() {
    let h = Harness::new().await;
    let p = h.create("p", None).await;
    let a = h.create("a", None).await;
    let b = h.create("b", None).await;
    let r = h.move_note(&a, Value::Null, i64::MAX, 1).await;
    assert_eq!(r.0, StatusCode::OK);
    assert_eq!(r.2["position"], 2);
    assert_eq!(h.order(None).await, ["p", "b", "a"]);
    h.delete(&a, 2).await;
    h.delete(&b, 1).await;
    let r = h
        .restore(
            &a,
            json!({"expected_version":3,"restore_parent":{"parent_id":p["id"]}}),
        )
        .await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.2["parent_id"], p["id"]);
    assert_eq!(r.2["position"], 0);
    assert_eq!(h.order(Some(&p)).await, ["a"]);
    let events: Vec<Value> = sqlx::query_scalar("SELECT payload FROM outbox WHERE payload->>'source_id'=$1 ORDER BY (payload->>'content_version')::bigint")
        .bind(a["id"].as_str().unwrap()).fetch_all(&h.db.pool).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["created", "moved", "deleted", "restored"]
    );
    assert_eq!(events.last().unwrap()["content_version"], 4);
}

// A persisted historical chain must be checked beyond a deleted intermediate.
#[tokio::test]
async fn t24_restore_checks_all_stored_ancestors() {
    let h = Harness::new().await;
    let a = h.create("a", None).await;
    let b = h.create("b", a["id"].as_str()).await;
    let c = h.create("c", b["id"].as_str()).await;
    h.delete(&c, 1).await;
    h.delete(&b, 1).await;
    h.delete(&a, 1).await;
    // A legacy/inconsistent live leaf is a defensive fixture: its stored parent
    // remains b, so looking only at live ancestors would miss a -> c -> b -> a.
    sqlx::query("UPDATE notes SET deleted_at=NULL WHERE id=$1")
        .bind(Uuid::parse_str(c["id"].as_str().unwrap()).unwrap())
        .execute(&h.db.pool)
        .await
        .unwrap();
    let before = h.state().await;
    let events = h.events().await;
    assert_eq!(
        h.restore(
            &a,
            json!({"expected_version":2,"restore_parent":{"parent_id":c["id"]}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(h.state().await, before);
    assert_eq!(h.events().await, events);
}
