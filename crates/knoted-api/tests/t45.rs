use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use knoted_api::build_router;
use knoted_core::{
    auth::{self, sessions},
    config::AuthConfig,
    context::AppContext,
    db::testing::TestDb,
    files::safe_fs,
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;
#[tokio::test]
async fn t45_http_relink_authenticated_csrf_and_target_etag() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let app = build_router(ctx.clone());
    let root = tempfile::tempdir_in("/data/knoted-dev/tasks/t45/tmp").unwrap();
    let root_id = Uuid::new_v4();
    let identity = safe_fs::metadata(safe_fs::open_root(root.path()).unwrap())
        .unwrap()
        .identity;
    sqlx::query("INSERT INTO file_roots(id,configured_path,identity) VALUES($1,$2,$3)")
        .bind(root_id)
        .bind(root.path().to_str().unwrap())
        .bind(sqlx::types::Json(identity))
        .execute(ctx.pool())
        .await
        .unwrap();
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    for (id, path, note) in [(old, "old.txt", Some("note")), (new, "new.txt", None)] {
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
            .bind(id)
            .execute(ctx.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,note,observation,last_seen_at) VALUES($1,$2,$3,1,2,3,4,5,1,repeat('a',64),7,11,'present',$4,'{}',now())").bind(id).bind(root_id).bind(path).bind(note).execute(ctx.pool()).await.unwrap();
    }
    let mut tx = ctx.pool().begin().await.unwrap();
    let owner = auth::ensure_owner(&mut tx).await.unwrap();
    let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let path = format!("/api/v1/files/{old}/relink");
    let body = json!({"new_source_id":new,"expected_version":11});
    let mut results = vec![];
    for (auth, csrf, payload) in [
        (false, false, body.clone()),
        (true, false, body.clone()),
        (
            true,
            true,
            json!({"new_source_id":new,"expected_version":11,"confirm":true}),
        ),
        (
            true,
            true,
            json!({"new_source_id":old,"expected_version":11}),
        ),
        (true, true, body.clone()),
        (true, true, body),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json");
        if auth {
            req = req.header(
                "cookie",
                format!(
                    "knoted_session={}; knoted_csrf={}",
                    session.token, session.csrf_token
                ),
            );
        }
        if csrf {
            req = req
                .header("origin", AuthConfig::default().origin)
                .header("x-csrf-token", &session.csrf_token);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::from(payload.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let etag = response.headers().get("etag").cloned();
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        results.push((status, etag, value));
    }
    db.drop().await;
    assert_eq!(
        results.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::BAD_REQUEST,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::OK,
            StatusCode::CONFLICT
        ]
    );
    assert_eq!(results[4].1.as_ref().unwrap(), "\"12\"");
    assert_eq!(results[4].2["id"], new.to_string());
    assert_eq!(results[4].2["content_version"], 7);
    assert_eq!(results[4].2["name"], "new.txt");
    assert_eq!(results[5].2["current_version"], 12);
}
