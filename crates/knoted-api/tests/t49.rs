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
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn t49_1_anonymous_reads_rejected() {
    let db = TestDb::create().await.unwrap();
    let app = build_router(AppContext::new(db.pool.clone()));
    let mut statuses = vec![];
    for path in [
        "/api/v1/files".to_owned(),
        format!("/api/v1/files/{}", Uuid::new_v4()),
        format!("/api/v1/files/{}/content", Uuid::new_v4()),
    ] {
        statuses.push(
            app.clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
        );
    }
    db.drop().await;
    assert_eq!(statuses, vec![StatusCode::UNAUTHORIZED; 3]);
}

#[tokio::test]
async fn t49_3_patch_metadata_cas_and_contract() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let app = build_router(ctx.clone());
    let root = Uuid::new_v4();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO file_roots(id,configured_path) VALUES($1,'/unavailable')")
        .bind(root)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,last_seen_at) VALUES($1,$2,'safe.txt',1,2,3,4,5,1,repeat('a',64),7,11,'present','{}',now())").bind(id).bind(root).execute(ctx.pool()).await.unwrap();
    let mut tx = ctx.pool().begin().await.unwrap();
    let owner = auth::ensure_owner(&mut tx).await.unwrap();
    let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let cookie = format!(
        "knoted_session={}; knoted_csrf={}",
        session.token, session.csrf_token
    );
    let mut outcomes = vec![];
    for (method, path, csrf, payload) in [
        ("GET", "/api/v1/files".into(), false, Value::Null),
        ("GET", format!("/api/v1/files/{id}"), false, Value::Null),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            false,
            json!({"expected_version":11,"note":"x"}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":11}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":11,"relative_path":"other"}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":11,"note":"  原文  ","tag_ids":[]}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":11,"note":null}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":12,"note":"  原文  "}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":12,"note":null}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":13,"tag_ids":null}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":13,"note":"a\0b"}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":0,"note":"x"}),
        ),
        (
            "PATCH",
            format!("/api/v1/files/{id}"),
            true,
            json!({"expected_version":13,"note":"partial","tag_ids":[Uuid::new_v4()]}),
        ),
        ("GET", format!("/api/v1/files/{id}"), false, Value::Null),
    ] {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", &cookie)
            .header("content-type", "application/json");
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
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        outcomes.push((
            status,
            etag,
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        ));
    }
    db.drop().await;
    assert_eq!(
        outcomes.iter().map(|r| r.0.as_u16()).collect::<Vec<_>>(),
        [
            200, 200, 403, 400, 400, 200, 409, 200, 200, 400, 422, 422, 422, 200
        ]
    );
    assert_eq!(outcomes[5].2["note"], "  原文  ");
    assert_eq!(outcomes[5].2["sha256"], "a".repeat(64));
    assert_eq!(outcomes[5].2["content_version"], 7);
    assert_eq!(outcomes[5].1.as_ref().unwrap(), "\"12\"");
    assert_eq!(outcomes[6].2["current_version"], 12);
    assert_eq!(outcomes[7].2["version"], 12);
    assert_eq!(outcomes[8].2["version"], 13);
    assert!(outcomes[8].2["note"].is_null());
    assert!(outcomes[13].2["note"].is_null());
    assert_eq!(outcomes[13].2["version"], 13);
}

use knoted_core::{
    config::FilesConfig,
    files::{Sha256Hex, safe_fs},
};
async fn fixture() -> (TestDb, AppContext, tempfile::TempDir, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(dir.path().into()),
        ..Default::default()
    });
    std::fs::write(dir.path().join("a.txt"), b"source bytes").unwrap();
    let root = Uuid::new_v4();
    let scan = Uuid::new_v4();
    let id = Uuid::new_v4();
    let identity = safe_fs::metadata(safe_fs::open_root(dir.path()).unwrap())
        .unwrap()
        .identity;
    let m = safe_fs::metadata(std::fs::File::open(dir.path().join("a.txt")).unwrap()).unwrap();
    sqlx::query("INSERT INTO file_roots(id,configured_path,identity,generation,last_complete_generation) VALUES($1,$2,$3,1,1)").bind(root).bind(dir.path().to_str().unwrap()).bind(sqlx::types::Json(identity)).execute(ctx.pool()).await.unwrap();
    let job: Uuid = sqlx::query_scalar("INSERT INTO jobs(kind) VALUES('t49.fixture') RETURNING id")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_scans(id,root_id,generation,token,job_id,lease_token,complete,finished_at) VALUES($1,$2,1,$3,$4,$3,true,now())").bind(scan).bind(root).bind(Uuid::new_v4()).bind(job).execute(ctx.pool()).await.unwrap();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,last_seen_at,content_fresh,captured_observation,captured_observation_version,observation_version,captured_scan_id,input_object_id) VALUES($1,$2,'a.txt',$3,$4,$5,$6,$7,$8,$9,7,11,'present',$10,now(),true,$10,1,1,$11,'fixture')")
 .bind(id).bind(root).bind(m.identity.device as i64).bind(m.identity.inode as i64).bind(m.size as i64).bind(m.mtime_ns).bind(m.ctime_ns).bind(m.nlink as i64).bind(Sha256Hex::digest(b"source bytes").as_str()).bind(sqlx::types::Json(m)).bind(scan).execute(ctx.pool()).await.unwrap();
    (db, ctx, dir, id)
}

#[tokio::test]
async fn t49_content_authenticated_bytes_and_headers() {
    let (db, ctx, _dir, id) = fixture().await;
    let app = build_router(ctx.clone());
    let mut tx = ctx.pool().begin().await.unwrap();
    let owner = auth::ensure_owner(&mut tx).await.unwrap();
    let session = sessions::create_session(&mut tx, owner.user.id, ctx.now(), None, true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/files/{id}/content"))
                .header("cookie", format!("knoted_session={}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    db.drop().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&bytes[..], b"source bytes");
    assert_eq!(headers["etag"], "\"11\"");
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(headers["content-length"], "12");
    assert_eq!(headers["cache-control"], "private, no-store");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
}
