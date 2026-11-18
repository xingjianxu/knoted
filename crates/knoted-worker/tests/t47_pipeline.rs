//! Real scan -> capture -> Python OCR -> fenced success row in isolated state.
use knoted_core::{
    config::{FilesConfig, IntelligenceConfig},
    context::AppContext,
    db::testing::TestDb,
    files::{extraction, scanner},
    intelligence::IntelligenceClient,
    jobs,
};
use knoted_worker::{
    handlers::scan::Dispatcher,
    runner::{StepOutcome, WorkerRunner},
};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Service {
    child: Child,
    root: PathBuf,
}
impl Drop for Service {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .args(["-TERM", &self.child.id().to_string()])
                .status();
        }
        let deadline = Instant::now() + Duration::from_secs(25);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn t47_real_scan_capture_ocr_publication_and_unchanged_no_retry() {
    let db = TestDb::create().await.unwrap();
    let root = std::env::temp_dir().join(format!("t47-pipeline-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let staging = root.join("staging");
    std::fs::create_dir_all(&source).unwrap();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::copy(
        repository.join("tests/fixtures/ocr/sample_zh_en.png"),
        source.join("sample.png"),
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}");
    let python = std::env::var("KNOTED_TEST_PYTHON").unwrap_or_else(|_| {
        std::env::var("UV_PROJECT_ENVIRONMENT")
            .map(|p| format!("{p}/bin/python"))
            .unwrap_or_else(|_| {
                repository
                    .join("services/intelligence/.venv/bin/python")
                    .to_string_lossy()
                    .into_owned()
            })
    });
    let child = Command::new(python)
        .args([
            "-I",
            "-B",
            "-c",
            "from knoted_intelligence.app import main;main()",
        ])
        .env("KNOTED_INTELLIGENCE_HOST", "127.0.0.1")
        .env("KNOTED_INTELLIGENCE_PORT", port.to_string())
        .env("KNOTED_INTELLIGENCE_STATE_DIR", root.join("state"))
        .env("KNOTED_INTELLIGENCE_STAGING_DIR", &staging)
        .env("KNOTED_INTELLIGENCE_DOCLING_DEVICE", "cpu")
        .env("CUDA_VISIBLE_DEVICES", "")
        .env("KNOTED_EMBED_GPU_UUID", "")
        .env_remove("KNOTED_INTELLIGENCE_CHILD_HOLD_MS")
        .env_remove("KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM")
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::File::create(root.join("api.log")).unwrap(),
        ))
        .spawn()
        .unwrap();
    let mut service = Service { child, root };
    let client = IntelligenceClient::new(&url).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while client.health().await.is_err() {
        assert!(service.child.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ctx = AppContext::new(db.pool.clone())
        .with_files(FilesConfig {
            enabled: true,
            root: Some(source),
            staging_root: Some(staging),
            ..Default::default()
        })
        .with_intelligence(IntelligenceConfig { base_url: url });
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t47-pipeline", Dispatcher { ctx: ctx.clone() });
    for scan in 0..2 {
        let mut tx = db.pool.begin().await.unwrap();
        jobs::enqueue(&mut tx, jobs::JobSpec::new(scanner::SCAN_JOB_KIND))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        loop {
            let step = tokio::time::timeout(Duration::from_secs(180), runner.step())
                .await
                .unwrap()
                .unwrap();
            match step {
                StepOutcome::Idle => break,
                StepOutcome::Finished { job, outcome } => assert!(
                    matches!(outcome, jobs::ExecutionOutcome::Succeeded),
                    "{}: {outcome:?}",
                    job.kind
                ),
                other => panic!("{other:?}"),
            }
        }
        let (id, blocks): (uuid::Uuid, String) =
            sqlx::query_as("SELECT execution_id,blocks::text FROM file_extractions")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(blocks.contains("中文"), "{blocks}");
        let attempts = extraction::load_attempts(
            &db.pool,
            sqlx::query_scalar("SELECT job_id FROM file_extractions")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(id, attempts[0].request.execution_id);
        client
            .get_execution(id)
            .await
            .unwrap()
            .verify_resource_exit(&attempts[0].request)
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "scan {scan} must not repeat unchanged success");
    }
    // Start the actual Worker binary against this throw-away PG schema and
    // stop it only after the real compute child has entered its bootstrap.
    let worker_root = service.root.join("worker");
    std::fs::create_dir(&worker_root).unwrap();
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(
        schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    let mut database_url = knoted_core::db::testing::test_database_url();
    database_url.push_str(if database_url.contains('?') { "&" } else { "?" });
    database_url.push_str(&format!("options=-csearch_path%3D{schema},public"));
    let config = worker_root.join("worker.toml");
    std::fs::write(&config, format!(
        "[database]\nurl={database_url:?}\n[files]\nenabled=true\nroot={:?}\nstaging_root={:?}\n[intelligence]\nbase_url={:?}\n",
        ctx.files().root.as_ref().unwrap(), ctx.files().staging_root.as_ref().unwrap(), ctx.intelligence().base_url
    )).unwrap();
    std::fs::copy(
        repository.join("tests/fixtures/ocr/sample_zh_en.png"),
        ctx.files().root.as_ref().unwrap().join("stop.png"),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_knoted-worker"))
        .args(["--config", config.to_str().unwrap()])
        .env_remove("KNOTED_DATABASE_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::File::create(worker_root.join("worker.log")).unwrap(),
        ))
        .spawn()
        .unwrap();
    let mut worker = Service {
        child,
        root: worker_root,
    };
    let limit = Instant::now() + Duration::from_secs(60);
    let attempt = loop {
        assert!(
            worker.child.try_wait().unwrap().is_none(),
            "worker exited before compute"
        );
        assert!(Instant::now() < limit, "worker did not reach compute");
        let id: Option<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM jobs WHERE kind='file.extract' AND state='running' ORDER BY created_at DESC LIMIT 1").fetch_optional(&db.pool).await.unwrap();
        if let Some(id) = id
            && let Some(attempt) = extraction::load_attempts(&db.pool, id)
                .await
                .unwrap()
                .first()
            && service
                .root
                .join(format!(
                    "staging/output/{}/child-status.json",
                    attempt.request.execution_id
                ))
                .exists()
        {
            break attempt.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        Command::new("kill")
            .args(["-TERM", &worker.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let limit = Instant::now() + Duration::from_secs(65);
    loop {
        if let Some(status) = worker.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < limit);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    client
        .get_execution(attempt.request.execution_id)
        .await
        .unwrap()
        .verify_resource_exit(&attempt.request)
        .unwrap();
    drop(worker);
    drop(service);
    db.drop().await;
}
