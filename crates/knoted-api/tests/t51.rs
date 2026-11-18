use std::{fs, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, request::Builder},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, TimeZone, Utc};
use knoted_api::{build_router_with_auth, logging::LogBuffer};
use knoted_core::{
    auth::{
        self,
        sessions::{self, CreatedSession},
    },
    config::{AppConfig, FilesConfig},
    context::{AppContext, FixedClock},
    db::testing::TestDb,
    files::{Sha256Hex, safe_fs},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const PUBLIC: &str = "https://knoted.example.test:18443";
const DOCS: &str = "https://docs.example.test:19443";
const ORIGINAL: &[u8] = b"fixed file fixture";

struct Fixture {
    db: TestDb,
    dir: tempfile::TempDir,
    ctx: AppContext,
    clock: Arc<FixedClock>,
    root: Uuid,
    scan: Uuid,
    // Known synthetic test input, never loaded from any shared location.
    signing_fixture: [u8; 32],
}

impl Fixture {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let files = dir.path().join("sources");
        fs::create_dir(&files).unwrap();
        let signing_fixture = [b'a'; 32];
        fs::write(
            dir.path().join("test-signing-input"),
            [signing_fixture.as_slice(), b"\n"].concat(),
        )
        .unwrap();
        let clock = Arc::new(FixedClock::new(
            Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap(),
        ));
        let ctx = AppContext::new(db.pool.clone())
            .with_clock(clock.clone())
            .with_files(FilesConfig {
                enabled: true,
                root: Some(files.clone()),
                ..Default::default()
            });
        let root = Uuid::new_v4();
        let scan = Uuid::new_v4();
        let identity = safe_fs::metadata(safe_fs::open_root(&files).unwrap())
            .unwrap()
            .identity;
        sqlx::query("INSERT INTO file_roots(id,configured_path,identity,generation,last_complete_generation) VALUES($1,$2,$3,1,1)")
            .bind(root).bind(files.to_str().unwrap()).bind(sqlx::types::Json(identity)).execute(&db.pool).await.unwrap();
        let job: Uuid =
            sqlx::query_scalar("INSERT INTO jobs(kind) VALUES('t51.fixture') RETURNING id")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO file_scans(id,root_id,generation,token,job_id,lease_token,complete,finished_at) VALUES($1,$2,1,$3,$4,$3,true,now())")
            .bind(scan).bind(root).bind(Uuid::new_v4()).bind(job).execute(&db.pool).await.unwrap();
        Self {
            db,
            dir,
            ctx,
            clock,
            root,
            scan,
            signing_fixture,
        }
    }

    fn source_path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join("sources").join(name)
    }

    fn app(&self, configured: bool) -> Router {
        let auth = format!("[auth]\nrp_id = \"knoted.example.test\"\norigin = \"{PUBLIC}\"\n");
        let path = self.dir.path().join("api-test.toml");
        let config = if configured {
            let secret_file = self.dir.path().join("test-signing-input");
            format!(
                "{auth}\n[office]\ndocument_server_origin = \"{DOCS}\"\njwt_secret_file = \"{}\"\n",
                secret_file.display()
            )
        } else {
            auth
        };
        fs::write(&path, config).unwrap();
        let config = AppConfig::load(path).unwrap();
        let ctx = self
            .ctx
            .clone()
            .with_office(config.office.as_ref(), config.auth());
        build_router_with_auth(ctx, config.auth)
    }

    async fn session(&self) -> CreatedSession {
        let mut tx = self.db.pool.begin().await.unwrap();
        let owner = auth::ensure_owner(&mut tx).await.unwrap();
        let session = sessions::create_session(&mut tx, owner.user.id, self.ctx.now(), None, true)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        session
    }

    async fn add_file(&self, name: &str, size: u64) -> Uuid {
        let path = self.source_path(name);
        let id = Uuid::new_v4();
        if !name.contains('/') {
            fs::write(&path, ORIGINAL).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(size)
                .unwrap();
        }
        let m = safe_fs::metadata(fs::File::open(&path).unwrap()).unwrap();
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
            .bind(id)
            .execute(&self.db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,last_seen_at,content_fresh,captured_observation,captured_observation_version,observation_version,captured_scan_id,input_object_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,7,11,'present',$11,now(),true,$11,1,1,$12,'fixture')")
            .bind(id).bind(self.root).bind(name).bind(m.identity.device as i64).bind(m.identity.inode as i64)
            .bind(m.size as i64).bind(m.mtime_ns).bind(m.ctime_ns).bind(m.nlink as i64)
            .bind(Sha256Hex::digest(ORIGINAL).as_str()).bind(sqlx::types::Json(m)).bind(self.scan)
            .execute(&self.db.pool).await.unwrap();
        id
    }
}

fn browser(req: Builder, session: Option<&CreatedSession>, csrf: bool) -> Builder {
    let Some(session) = session else { return req };
    let req = req.header(
        "cookie",
        format!(
            "knoted_session={}; knoted_csrf={}",
            session.token, session.csrf_token
        ),
    );
    if csrf {
        req.header("origin", PUBLIC)
            .header("x-csrf-token", &session.csrf_token)
    } else {
        req
    }
}

async fn create(
    app: &Router,
    file: Uuid,
    version: i64,
    session: Option<&CreatedSession>,
    csrf: bool,
) -> Response {
    app.clone()
        .oneshot(
            browser(
                Request::post(format!("/api/v1/files/{file}/office-preview")),
                session,
                csrf,
            )
            .header("host", "ignored.example.test")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"expected_content_version": version}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap()
}

async fn created(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap()).unwrap()
}

fn preview_id(preview: &Value) -> Uuid {
    preview["preview_id"].as_str().unwrap().parse().unwrap()
}

fn source_path(id: Uuid) -> String {
    format!("/api/v1/office-preview-sources/{id}")
}

fn outbox_claims(uri: &str, issued: i64, expires: i64) -> Value {
    json!({"payload":{"url":format!("{PUBLIC}{uri}")},"iat":issued,"exp":expires})
}

fn signed_jwt(key: &[u8], header: &[u8], claims: &[u8]) -> String {
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(claims)
    );
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    let tag = ring::hmac::sign(&key, input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(tag))
}

const JWT_HEADER: &[u8] = br#"{"alg":"HS256","typ":"JWT"}"#;

fn bearer(f: &Fixture, path: &str) -> String {
    let now = f.ctx.now().timestamp();
    signed_jwt(
        &f.signing_fixture,
        JWT_HEADER,
        outbox_claims(path, now, now + 300).to_string().as_bytes(),
    )
}

async fn send(app: &Router, req: Builder) -> Response {
    app.clone()
        .oneshot(
            req.body(Body::empty())
                .expect("request fixture must be valid"),
        )
        .await
        .unwrap()
}

async fn fetch(f: &Fixture, app: &Router, preview: &Value, range: bool) -> Response {
    let path = source_path(preview_id(preview));
    let req = Request::get(&path).header("authorization", format!("Bearer {}", bearer(f, &path)));
    let req = if range {
        req.header("range", "bytes=0-1")
    } else {
        req
    };
    send(app, req).await
}

async fn revoke(
    app: &Router,
    file: Uuid,
    preview: Uuid,
    session: &CreatedSession,
    csrf: bool,
) -> StatusCode {
    app.clone()
        .oneshot(
            browser(
                Request::delete(format!(
                    "/api/v1/files/{file}/office-preview?preview_id={preview}"
                )),
                Some(session),
                csrf,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn t51_1_source_expires_and_revoke_is_idempotent() {
    let f = Fixture::new().await;
    let file = f.add_file("slides.pptx", ORIGINAL.len() as u64).await;
    let app = f.app(true);
    let session = f.session().await;
    let preview = created(create(&app, file, 7, Some(&session), true).await).await;
    f.clock.advance(Duration::seconds(119));
    let response = fetch(&f, &app, &preview, true).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Range must not select a partial response"
    );
    let headers = response.headers();
    assert_eq!(
        headers["content-type"],
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    );
    assert_eq!(headers["cache-control"], "private, no-store");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(!headers.contains_key("content-range"));
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        ORIGINAL
    );
    let last_seen: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE id=$1")
            .bind(session.session.id)
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
    assert_eq!(
        last_seen, session.session.last_seen_at,
        "source GET cannot renew idle time"
    );
    f.clock.advance(Duration::seconds(1));
    assert_eq!(fetch(&f, &app, &preview, false).await.status(), 404);
    assert_eq!(
        revoke(&app, file, preview_id(&preview), &session, true).await,
        204
    );
    let preview = created(create(&app, file, 7, Some(&session), true).await).await;
    for _ in 0..2 {
        assert_eq!(
            revoke(&app, file, preview_id(&preview), &session, true).await,
            204
        );
    }
    assert_eq!(fetch(&f, &app, &preview, false).await.status(), 404);
}

#[tokio::test]
async fn t51_2_outbox_scope_is_process_preview_file_and_session() {
    let f = Fixture::new().await;
    fs::create_dir(f.dir.path().join("outside")).unwrap();
    fs::write(f.dir.path().join("outside/outside.docx"), ORIGINAL).unwrap();
    std::os::unix::fs::symlink(f.dir.path().join("outside"), f.source_path("alias")).unwrap();
    let outside = f
        .add_file("alias/outside.docx", ORIGINAL.len() as u64)
        .await;
    let file = f.add_file("report.docx", ORIGINAL.len() as u64).await;
    let app = f.app(true);
    let owner = f.session().await;
    let other = f.session().await;
    let a = created(create(&app, file, 7, Some(&owner), true).await).await;
    let b = created(create(&app, file, 7, Some(&owner), true).await).await;
    let path = source_path(preview_id(&a));
    let jwt = bearer(&f, &path);
    let auth = format!("Bearer {jwt}");
    // Positive control before the negatives: 404 from a disabled or absent
    // endpoint must not make the adversarial fixtures pass vacuously.
    assert_eq!(
        send(&app, Request::get(&path).header("authorization", &auth))
            .await
            .status(),
        200
    );
    let browser_jwt = a["config"]["token"].as_str().unwrap();
    let log = LogBuffer::new();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer({
            let log = log.clone();
            move || log.clone()
        })
        .finish();
    let _log_guard = tracing::subscriber::set_default(subscriber);
    let now = f.ctx.now().timestamp();
    let current = outbox_claims(&path, now, now + 300);
    let sign = |claims: &Value| {
        signed_jwt(
            &f.signing_fixture,
            JWT_HEADER,
            claims.to_string().as_bytes(),
        )
    };
    let mut foreign = current.clone();
    foreign["payload"]["url"] = json!(format!("https://other.example.test:18443{path}"));
    let mut missing_iat = current.clone();
    missing_iat.as_object_mut().unwrap().remove("iat");
    let mut invalid_purpose = current.clone();
    invalid_purpose["payload"]["operation"] = json!("edit");
    let invalid_jwts = [
        ("browser JWT", browser_jwt.to_owned()),
        (
            "wrong signature",
            signed_jwt(&[b'b'; 32], JWT_HEADER, current.to_string().as_bytes()),
        ),
        (
            "wrong algorithm",
            signed_jwt(
                &f.signing_fixture,
                br#"{"alg":"HS512","typ":"JWT"}"#,
                current.to_string().as_bytes(),
            ),
        ),
        ("wrong origin", sign(&foreign)),
        ("wrong preview", bearer(&f, &source_path(preview_id(&b)))),
        (
            "claimed query",
            sign(&outbox_claims(&format!("{path}?x=1"), now, now + 300)),
        ),
        (
            "claimed fragment",
            sign(&outbox_claims(&format!("{path}#x"), now, now + 300)),
        ),
        ("expired", sign(&outbox_claims(&path, now - 300, now))),
        (
            "future issued",
            sign(&outbox_claims(&path, now + 1, now + 300)),
        ),
        ("missing iat", sign(&missing_iat)),
        ("wrong operation", sign(&invalid_purpose)),
        (
            "malformed claims",
            signed_jwt(&f.signing_fixture, JWT_HEADER, b"not-json"),
        ),
        ("malformed compact JWT", "one.two.three.four".to_owned()),
        ("noncanonical padding", format!("{jwt}=")),
        ("unbounded token", "a".repeat(8193)),
    ];
    let mut cases = vec![
        ("no auth", Request::get(&path)),
        (
            "cookie only",
            browser(Request::get(&path), Some(&owner), false),
        ),
        (
            "wrong scheme",
            Request::get(&path).header("authorization", format!("Basic {jwt}")),
        ),
        (
            "duplicate auth",
            Request::get(&path)
                .header("authorization", &auth)
                .header("authorization", &auth),
        ),
        (
            "combined auth",
            Request::get(&path).header("authorization", format!("{auth}, {auth}")),
        ),
        (
            "legacy query",
            Request::get(format!("{path}?token=synthetic-old-format")),
        ),
        (
            "query despite auth",
            Request::get(format!("{path}?x=1")).header("authorization", &auth),
        ),
        (
            "empty query",
            Request::get(format!("{path}?")).header("authorization", &auth),
        ),
    ];
    cases.extend(invalid_jwts.iter().map(|(label, jwt)| {
        (
            *label,
            Request::get(&path).header("authorization", format!("Bearer {jwt}")),
        )
    }));
    for (label, request) in cases {
        let response = send(&app, request).await;
        assert_eq!(response.status(), 404, "{label}");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "not_found");
        assert!(
            body["message"].as_str() == Some("preview source not found"),
            "same fixed message for {label}"
        );
    }
    let unknown = source_path(Uuid::new_v4());
    assert_eq!(
        send(
            &app,
            Request::get(&unknown)
                .header("authorization", format!("Bearer {}", bearer(&f, &unknown)))
        )
        .await
        .status(),
        404
    );
    assert_eq!(fetch(&f, &f.app(true), &a, false).await.status(), 404);
    assert_eq!(revoke(&app, file, preview_id(&a), &other, true).await, 204);
    assert_eq!(
        revoke(&app, outside, preview_id(&a), &owner, true).await,
        204
    );
    assert_eq!(fetch(&f, &app, &a, false).await.status(), 200);
    assert_eq!(revoke(&app, file, preview_id(&a), &owner, true).await, 204);
    assert_eq!(fetch(&f, &app, &a, false).await.status(), 404);
    assert_eq!(fetch(&f, &app, &b, false).await.status(), 200);
    let logs = log.contents();
    assert!(logs.contains(&path), "the trace retains the path");
    assert!(
        !logs.contains(&jwt)
            && !logs.contains(browser_jwt)
            && !logs.contains("synthetic-old-format")
            && invalid_jwts.iter().all(|(_, jwt)| !logs.contains(jwt)),
        "the trace must not record JWTs or queries"
    );
    assert_eq!(
        create(&app, outside, 7, Some(&owner), true).await.status(),
        409
    );
}

#[tokio::test]
async fn t51_3_preview_keeps_original_bytes_and_rejects_stale_version() {
    let f = Fixture::new().await;
    let file = f.add_file("workbook.XLSX", ORIGINAL.len() as u64).await;
    let app = f.app(true);
    let session = f.session().await;
    let preview = created(create(&app, file, 7, Some(&session), true).await).await;
    fs::write(f.source_path("workbook.XLSX"), b"replacement").unwrap();
    assert_eq!(
        create(&app, file, 7, Some(&session), true).await.status(),
        409,
        "on-disk change before scan"
    );
    sqlx::query("UPDATE file_sources SET content_version=8 WHERE id=$1")
        .bind(file)
        .execute(&f.db.pool)
        .await
        .unwrap();
    assert_eq!(
        create(&app, file, 7, Some(&session), true).await.status(),
        409,
        "selected version already superseded"
    );
    let response = fetch(&f, &app, &preview, false).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        ORIGINAL
    );
}

#[tokio::test]
async fn t51_4_concurrent_admission_counts_unconsumed_source_body() {
    let f = Fixture::new().await;
    let file = f.add_file("report.docx", ORIGINAL.len() as u64).await;
    let app = f.app(true);
    let s = f.session().await;
    let (a, b, c) = tokio::join!(
        create(&app, file, 7, Some(&s), true),
        create(&app, file, 7, Some(&s), true),
        create(&app, file, 7, Some(&s), true),
    );
    let mut responses = [a, b, c];
    responses.sort_by_key(Response::status);
    let [a, b, rejected] = responses;
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    let a = created(a).await;
    let b = created(b).await;
    assert!(
        a["config"]["document"]["key"] != b["config"]["document"]["key"],
        "key must be unique per preview"
    );
    let pending = fetch(&f, &app, &a, false).await;
    assert_eq!(pending.status(), 200);
    assert_eq!(revoke(&app, file, preview_id(&a), &s, true).await, 204);
    assert_eq!(create(&app, file, 7, Some(&s), true).await.status(), 429);
    // Even after the frame is polled the Bytes owner is part of the bound.
    let bytes = to_bytes(pending.into_body(), 1024).await.unwrap();
    assert_eq!(create(&app, file, 7, Some(&s), true).await.status(), 429);
    assert_eq!(&bytes[..], ORIGINAL);
    drop(bytes);
    let _c = created(create(&app, file, 7, Some(&s), true).await).await;
    f.clock.advance(Duration::seconds(120));
    let _d = created(create(&app, file, 7, Some(&s), true).await).await;
}

#[tokio::test]
async fn t51_5_requires_session_csrf_config_and_signs_read_only() {
    let f = Fixture::new().await;
    let file = f.add_file("report.xlsx", ORIGINAL.len() as u64).await;
    let unsupported = f.add_file("legacy.xls", ORIGINAL.len() as u64).await;
    let oversized = f.add_file("large.docx", 50 * 1024 * 1024 + 1).await;
    let app = f.app(true);
    let s = f.session().await;
    assert_eq!(create(&app, file, 7, None, false).await.status(), 401);
    assert_eq!(create(&app, file, 7, Some(&s), false).await.status(), 403);
    assert_eq!(
        create(&f.app(false), file, 7, Some(&s), true)
            .await
            .status(),
        503
    );
    assert_eq!(
        create(&app, unsupported, 7, Some(&s), true).await.status(),
        422
    );
    assert_eq!(
        create(&app, oversized, 7, Some(&s), true).await.status(),
        422
    );
    assert_eq!(create(&app, file, 0, Some(&s), true).await.status(), 422);
    let preview = created(create(&app, file, 7, Some(&s), true).await).await;
    assert_eq!(preview["document_server_origin"], DOCS);
    let expected_url = format!("{PUBLIC}{}", source_path(preview_id(&preview)));
    assert!(
        preview["config"]["document"]["url"].as_str() == Some(&expected_url),
        "the source URL must have only configured origin and preview path, with no credentials"
    );
    let expires: chrono::DateTime<Utc> = preview["expires_at"].as_str().unwrap().parse().unwrap();
    assert_eq!(expires, f.ctx.now() + Duration::seconds(120));
    let mut config = preview["config"].clone();
    let jwt = config.as_object_mut().unwrap().remove("token").unwrap();
    let jwt = jwt.as_str().unwrap();
    let (input, tag) = jwt.rsplit_once('.').unwrap();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &f.signing_fixture);
    assert!(
        ring::hmac::verify(
            &key,
            input.as_bytes(),
            &URL_SAFE_NO_PAD.decode(tag).unwrap()
        )
        .is_ok(),
        "signature must verify with known test input"
    );
    let (header, payload) = input.split_once('.').unwrap();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
    assert_eq!(header["alg"], "HS256");
    let mut claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(
        claims.as_object_mut().unwrap().remove("exp").unwrap(),
        expires.timestamp()
    );
    assert!(
        claims == config,
        "the signature must cover the exact SDK config"
    );
    assert_eq!(config["documentType"], "cell");
    assert_eq!(config["document"]["fileType"], "xlsx");
    for permission in [
        "edit",
        "review",
        "comment",
        "fillForms",
        "modifyFilter",
        "modifyContentControl",
        "chat",
        "download",
        "print",
        "copy",
        "changeHistory",
        "protect",
    ] {
        assert_eq!(config["document"]["permissions"][permission], false);
    }
    let editor = &config["editorConfig"];
    assert_eq!(editor["mode"], "view");
    assert_eq!(editor["customization"]["plugins"], false);
    assert_eq!(editor["customization"]["macros"], false);
    assert_eq!(editor["customization"]["macrosMode"], "disable");
    assert!(editor.get("callbackUrl").is_none());
    assert_eq!(
        revoke(&app, file, preview_id(&preview), &s, false).await,
        403
    );
    let mut conn = f.db.pool.acquire().await.unwrap();
    sessions::logout(&mut conn, s.session.user_id, s.session.id, f.ctx.now())
        .await
        .unwrap();
    drop(conn);
    assert_eq!(fetch(&f, &app, &preview, false).await.status(), 404);
    assert_eq!(create(&app, file, 7, Some(&s), true).await.status(), 401);
    fs::remove_file(f.dir.path().join("test-signing-input")).unwrap();
    assert_eq!(
        create(&f.app(true), file, 7, Some(&f.session().await), true)
            .await
            .status(),
        503
    );
}
