use knoted_api::admin::restore::RestoreCommand;

#[test]
fn t69_3_requires_explicit_archive_and_target() {
    assert!(RestoreCommand::parse(["--input", "/archive"]).is_err());
    assert!(RestoreCommand::parse(["--input", "/a", "--target", "/t", "--input", "/b"]).is_err());
    let command =
        RestoreCommand::parse(["--target", "/target.toml", "--input", "/archive"]).unwrap();
    assert_eq!(command.input.to_str(), Some("/archive"));
    assert_eq!(command.target.to_str(), Some("/target.toml"));
}

#[test]
fn t69_1_digest_mismatch_rejects_archive() {
    use knoted_api::admin::restore::verify_archive;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("assets")).unwrap();
    std::fs::write(root.join("database.dump"), b"PGDMP").unwrap();
    let manifest = serde_json::json!({
        "format_version": 1, "completed_at": "2026-01-01T00:00:00Z", "app_version":"test",
        "artifact_revision":null, "migrations":[], "assets":[],
        "database_dump":{"size":5, "sha256":"0".repeat(64)},
        "source":{"database":"src","rp_id":"localhost","origin":"https://localhost",
        "asset_root":"/src/assets","staging_root":null,"file_root":null,
        "intelligence_origin":"http://127.0.0.1:15603",
        "search":{"enabled":true,"origin":"http://127.0.0.1:15605","document_index":"doc","chunk_index":"chunk"}}
    });
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(verify_archive(root).is_err());
}

#[tokio::test]
async fn t69_2_retires_restored_auth_and_prevents_job_replay() {
    use knoted_core::{
        context::AppContext,
        db::{restore, testing::TestDb},
        jobs,
    };
    let fixture = TestDb::create().await.unwrap();
    let ctx = AppContext::new(fixture.pool.clone());
    let pool = ctx.pool();
    let user = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users(id) VALUES ($1)")
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sessions(id,user_id,token_hash,csrf_token_hash,idle_expires_at,absolute_expires_at)
         VALUES(gen_random_uuid(),$1,'a','b',now()+interval '1 hour',now()+interval '1 hour')",
    )
    .bind(user)
        .execute(pool)
        .await
        .unwrap();
    let job: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO jobs(kind,state,idempotency_key) VALUES ('test','queued','test') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    restore::retire_source_runtime(pool).await.unwrap();
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE revoked_at IS NULL")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(live, 0);
    assert!(jobs::service::retry_failed(&ctx, job).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM jobs WHERE id=$1")
            .bind(job)
            .fetch_one(pool)
            .await
            .unwrap(),
        "cancelled"
    );
    fixture.drop().await;
}

use std::{fs, path::Path, process::Command};

struct PrivatePostgres {
    directory: tempfile::TempDir,
    user: String,
}

impl PrivatePostgres {
    async fn start() -> Self {
        use knoted_core::db::testing;
        use std::io::Write;
        let url = testing::test_database_url();
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        assert!(options.get_database().unwrap().ends_with("_test"));
        let reference = sqlx::PgPool::connect(&url).await.unwrap();
        let extension_settings: Vec<String> = sqlx::query_scalar(
            "SELECT format('%I = %L',name,setting) FROM pg_settings
             WHERE name IN ('extension_control_path','dynamic_library_path')",
        )
        .fetch_all(&reference)
        .await
        .unwrap();
        reference.close().await;
        let directory = tempfile::tempdir().unwrap();
        let user = "knoted_restore_fixture".to_owned();
        let pg = directory.path().join("pg");
        assert!(
            Command::new("initdb")
                .arg("-D")
                .arg(&pg)
                .args([
                    "--username",
                    &user,
                    "--auth-local=trust",
                    "--auth-host=reject",
                    "--no-instructions"
                ])
                .output()
                .unwrap()
                .status
                .success()
        );
        let mut settings = fs::OpenOptions::new()
            .append(true)
            .open(pg.join("postgresql.conf"))
            .unwrap();
        writeln!(
            settings,
            "listen_addresses = ''\nunix_socket_directories = '{}'\n{}",
            directory.path().display(),
            extension_settings.join("\n")
        )
        .unwrap();
        assert!(
            Command::new("pg_ctl")
                .arg("-D")
                .arg(&pg)
                .arg("-l")
                .arg(directory.path().join("postgres.log"))
                .args(["-w", "start"])
                .output()
                .unwrap()
                .status
                .success()
        );
        Self { directory, user }
    }
    fn url(&self, database: &str) -> String {
        format!(
            "postgres://{}@localhost/{}?host={}",
            self.user,
            database,
            self.directory.path().display()
        )
    }
}

impl Drop for PrivatePostgres {
    fn drop(&mut self) {
        let _ = Command::new("pg_ctl")
            .arg("-D")
            .arg(self.directory.path().join("pg"))
            .args(["-m", "fast", "-w", "stop"])
            .output();
    }
}

fn cli(verb: &str, options: &[(&str, &Path)]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_knoted-api"));
    cmd.args(["admin", verb]);
    for (flag, path) in options {
        cmd.arg(flag).arg(path);
    }
    cmd.output().unwrap()
}

async fn cookie_status(pool: &sqlx::PgPool, token: &str) -> http::StatusCode {
    use axum::body::Body;
    use knoted_api::middleware::auth::SESSION_COOKIE;
    use tower::ServiceExt;
    knoted_api::build_router(knoted_core::context::AppContext::new(pool.clone()))
        .oneshot(
            http::Request::builder()
                .uri("/api/v1/auth/sessions")
                .header("cookie", format!("{SESSION_COOKIE}={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn t69_1_t69_2_t69_3_restore_real_dump_and_assets() {
    use knoted_core::{auth::sessions, db, files::staging::Sha256Hex};
    use uuid::Uuid;
    let pg = PrivatePostgres::start().await;
    let home = pg.directory.path();
    let control = sqlx::PgPool::connect(&pg.url("postgres")).await.unwrap();
    sqlx::query("CREATE DATABASE archive_source TEMPLATE template0")
        .execute(&control)
        .await
        .unwrap();
    let source = sqlx::PgPool::connect(&pg.url("archive_source"))
        .await
        .unwrap();
    db::migrate(&source).await.unwrap();
    let owner = Uuid::new_v4();
    let image = Uuid::new_v4();
    let payload = include_bytes!("../../../tests/fixtures/ocr/sample_zh_en.png");
    let image_root = home.join("original-images");
    fs::create_dir(&image_root).unwrap();
    let original = image_root.join(image.to_string());
    fs::write(&original, payload).unwrap();
    sqlx::query("INSERT INTO users(id) VALUES ($1)")
        .bind(owner)
        .execute(&source)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO assets(id,owner_id,storage_key,mime,size,sha256,state,created_at)
         VALUES($1,$2,$3,'image/png',$4,$5,'ready',now())",
    )
    .bind(image)
    .bind(owner)
    .bind(image.to_string())
    .bind(payload.len() as i64)
    .bind(Sha256Hex::digest(payload).to_string())
    .execute(&source)
    .await
    .unwrap();
    let note = Uuid::new_v4();
    let snapshot = Uuid::new_v4();
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES ($1,'note'),($2,'project')")
        .bind(note)
        .bind(project)
        .execute(&source)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO notes(id,title,document,position,created_at,updated_at,deleted_at)
         VALUES($1,'trash','{\"blocks\":[]}',0,now(),now(),now())",
    )
    .bind(note)
    .execute(&source)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO note_snapshots(snapshot_id,note_id,content_version,title,document,reason,created_at)
         SELECT $1,id,content_version,title,document,'flush',now() FROM notes WHERE id=$2")
        .bind(snapshot).bind(note)
        .execute(&source)
        .await
        .unwrap();
    sqlx::query("INSERT INTO asset_refs(asset_id,ref_kind,ref_id,note_id) VALUES ($1,'snapshot',$2,$3),($1,'trash',$3,$3)")
        .bind(image).bind(snapshot).bind(note)
        .execute(&source)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO projects(id,title,created_at,updated_at) VALUES($1,'projection',now(),now())",
    )
    .bind(project)
    .execute(&source)
    .await
    .unwrap();
    sqlx::query("INSERT INTO credential_tokens(id,purpose,token_hash,expires_at) VALUES(gen_random_uuid(),'recovery',$1,now()+interval '5 minutes')")
        .bind(Uuid::new_v4().as_bytes().as_slice())
        .execute(&source)
        .await
        .unwrap();
    sqlx::query("INSERT INTO ceremonies(id,purpose,user_id,state,expires_at) VALUES(gen_random_uuid(),'login',$1,'{}',now()+interval '5 minutes')")
        .bind(owner)
        .execute(&source)
        .await
        .unwrap();
    let session = sessions::create_session(
        &mut source.acquire().await.unwrap(),
        owner,
        chrono::Utc::now(),
        None,
        false,
    )
    .await
    .unwrap();
    let auth_status = cookie_status(&source, &session.token).await;
    assert_eq!(auth_status, http::StatusCode::OK);
    let source_config = home.join("source.toml");
    fs::write(
        &source_config,
        format!(
            "[database]
url = {:?}
[server]
bind = '127.0.0.1:17685'
[assets]
root = {:?}
[auth]
origin = 'https://source.knoted.test:15881'
rp_id = 'knoted.test'
",
            pg.url("archive_source"),
            image_root
        ),
    )
    .unwrap();
    let archive = home.join("archive");
    let out = cli(
        "backup",
        &[("--output", &archive), ("--config", &source_config)],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let target_root = home.join("installed");
    let credentials = home.join("credentials.toml");
    fs::write(
        &credentials,
        format!("[secrets]\nengine = '{}'", Uuid::new_v4()),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let engine_pending = pending.clone();
    let engine = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/indexes",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"results":[],"total":0}))
            }),
        );
        let app = app.route(
            "/tasks",
            axum::routing::get(move || {
                let count = u8::from(engine_pending.load(std::sync::atomic::Ordering::SeqCst));
                async move { axum::Json(serde_json::json!({"results":[], "total": count})) }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let target_config = home.join("destination.toml");
    let target = home.join("descriptor.toml");
    let available_api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let available_python = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = available_api.local_addr().unwrap();
    let python = available_python.local_addr().unwrap();
    drop((available_api, available_python));
    let config = format!(
        "[database]
url={:?}
[server]
bind='{bind}'
[assets]
root={:?}
[files]
staging_root={:?}
[auth]
origin='https://restore.knoted.test:15882'
rp_id='knoted.test'
[search]
enabled=true
base_url='http://{addr}'
index_uid='restored_documents'
rag_chunk_index_uid='restored_chunks'
api_key_credential='engine'
[credentials]
credential_file={:?}
[intelligence]
base_url='http://{python}'
",
        pg.url("isolated-test"),
        target_root.join("assets"),
        target_root.join("staging"),
        credentials
    );
    fs::write(&target_config, &config).unwrap();
    fs::write(
        &target,
        format!(
            "instance_root={:?}\napp_config={:?}\n",
            target_root, target_config
        ),
    )
    .unwrap();
    let restore = || cli("restore", &[("--input", &archive), ("--target", &target)]);
    let archive_image = archive.join("assets").join(image.to_string());
    let mut corrupt = payload.to_vec();
    corrupt[0] ^= 1;
    fs::write(&archive_image, corrupt).unwrap();
    assert!(!restore().status.success());
    assert!(!target_root.exists());
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM pg_database WHERE datname='isolated-test' OR datname LIKE 'knoted_install_%')",
    )
    .fetch_one(&control)
    .await
    .unwrap();
    assert!(!exists);
    fs::write(&archive_image, payload).unwrap();
    assert!(!restore().status.success());
    assert!(!target_root.exists());
    pending.store(false, std::sync::atomic::Ordering::SeqCst);
    for invalid_config in [
        config.replace(&pg.url("isolated-test"), &pg.url("archive_source")),
        config.replace(&python.to_string(), "127.0.0.1:15603"),
        config.replace(&addr.to_string(), "127.0.0.2:15605"),
        config.replace("restored_documents", "knoted_documents"),
        config.replace("restore.knoted.test:15882", "source.knoted.test:15881"),
        config.replace("restore.knoted.test:15882", "evilknoted.test:15882"),
    ] {
        fs::write(&target_config, invalid_config).unwrap();
        assert!(!restore().status.success());
        assert!(!target_root.exists());
    }
    fs::write(&target_config, &config).unwrap();
    let python_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fs::write(
        &target_config,
        config.replace(
            &python.to_string(),
            &python_listener.local_addr().unwrap().to_string(),
        ),
    )
    .unwrap();
    assert!(!restore().status.success());
    assert!(!target_root.exists());
    drop(python_listener);
    fs::write(&target_config, &config).unwrap();
    let server = std::net::TcpListener::bind(bind).unwrap();
    assert!(!restore().status.success());
    drop(server);
    fs::create_dir(&target_root).unwrap();
    assert!(!restore().status.success());
    fs::remove_dir(&target_root).unwrap();
    std::os::unix::fs::symlink(&image_root, &target_root).unwrap();
    assert!(!restore().status.success());
    fs::remove_file(&target_root).unwrap();
    assert_eq!(fs::read_dir(&image_root).unwrap().count(), 1);
    let descriptor = fs::read_to_string(&target).unwrap();
    let nested = image_root.join("new-instance");
    let inside = |text: &str| text.replace(target_root.to_str().unwrap(), nested.to_str().unwrap());
    fs::write(&target, inside(&descriptor)).unwrap();
    fs::write(&target_config, inside(&config)).unwrap();
    assert!(!restore().status.success());
    assert!(!nested.exists());
    fs::write(&target, descriptor).unwrap();
    fs::write(&target_config, &config).unwrap();
    let out = restore();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let restored = sqlx::PgPool::connect(&pg.url("isolated-test"))
        .await
        .unwrap();
    assert_eq!(
        cookie_status(&restored, &session.token).await,
        http::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        cookie_status(&source, &session.token).await,
        http::StatusCode::OK
    );
    assert_eq!(
        fs::read(target_root.join("assets").join(image.to_string())).unwrap(),
        payload
    );
    assert_eq!(fs::read(&original).unwrap(), payload);
    assert!(!restore().status.success());
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users")
            .fetch_one(&restored)
            .await
            .unwrap(),
        owner
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM asset_refs WHERE asset_id=$1 AND note_id=$2"
        )
        .bind(image)
        .bind(note)
        .fetch_one(&restored)
        .await
        .unwrap(),
        2
    );
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT FROM note_snapshots s JOIN notes n ON n.id=s.note_id
          WHERE s.snapshot_id=$1 AND n.deleted_at IS NOT NULL)"
        )
        .bind(snapshot)
        .fetch_one(&restored)
        .await
        .unwrap()
    );
    let auth_state = |pool: sqlx::PgPool| async move {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT (SELECT count(*) FROM credential_tokens WHERE consumed_at IS NULL),
             (SELECT count(*) FROM ceremonies WHERE consumed_at IS NULL)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(auth_state(restored.clone()).await, (0, 0));
    assert_eq!(auth_state(source.clone()).await, (1, 1));
    assert!(sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT FROM fulltext_publications p JOIN source_documents s ON s.id=p.document_id
               JOIN jobs j ON j.id=p.job_id WHERE s.source_id=$1 AND p.index_uid='restored_documents' AND j.state='queued')")
        .bind(project).fetch_one(&restored).await.unwrap());
    engine.abort();
}
