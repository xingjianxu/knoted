//! Real scan -> capture -> Python OCR -> fenced success row in isolated state.
use knoted_core::{
    config::{FilesConfig, IntelligenceConfig},
    context::AppContext,
    db::testing::TestDb,
    files::{ObjectId, Sha256Hex, extraction, office, scanner},
    intelligence::{ExecutionKind, ExecutionRequest, IntelligenceClient},
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
async fn scan_once(ctx: &AppContext, runner: &WorkerRunner<Dispatcher>) {
    let mut tx = ctx.pool().begin().await.unwrap();
    jobs::enqueue(&mut tx, jobs::JobSpec::new(scanner::SCAN_JOB_KIND))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(180), runner.step())
            .await
            .unwrap()
            .unwrap()
        {
            StepOutcome::Idle => break,
            StepOutcome::Finished { outcome, .. } => assert!(matches!(
                outcome,
                jobs::ExecutionOutcome::Succeeded | jobs::ExecutionOutcome::PermanentFailure { .. }
            )),
            other => panic!("{other:?}"),
        }
    }
}
#[tokio::test]
async fn t48_retired_docx_keeps_takeover_pending_without_exit_proof() {
    let db = TestDb::create().await.unwrap();
    let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ctx = AppContext::new(db.pool.clone())
        .with_files(FilesConfig {
            enabled: true,
            ..Default::default()
        })
        .with_intelligence(IntelligenceConfig {
            base_url: format!("http://{}", unavailable.local_addr().unwrap()),
        });
    let mut tx = db.pool.begin().await.unwrap();
    let mut spec = jobs::JobSpec::new(extraction::EXTRACT_JOB_KIND);
    spec.payload = format!(
        r#"{{"source_id":"{}","root_id":"{}","content_version":1,"input_sha256":"{}",
            "input_object_id":"old-input","size":17,"observation_version":1,
            "captured_scan_id":"{}","observation":{{}},
            "parser_revision":"office-ooxml-body-v2-hidden-slides-included-cache-only:docx"}}"#,
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4(),
        Sha256Hex::digest(b"example old input"),
        uuid::Uuid::new_v4()
    )
    .parse()
    .unwrap();
    jobs::enqueue(&mut tx, spec).await.unwrap();
    tx.commit().await.unwrap();
    let previous = jobs::claim(&db.pool, "before-upgrade")
        .await
        .unwrap()
        .unwrap();
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("old-input").unwrap(),
        input_sha256: Sha256Hex::digest(b"example old input"),
        source_version: 1,
        deadline: ctx.now() + Duration::from_secs(900),
        options_revision: office::OPTIONS_REVISION.into(),
    };
    extraction::register_attempt(&db.pool, &previous, request.clone())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE jobs SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(previous.job_id)
    .execute(&db.pool)
    .await
    .unwrap();
    // Hold the port until context selection is complete, but run no server.
    drop(unavailable);
    let runner = WorkerRunner::with_context(ctx.clone(), "after-upgrade", Dispatcher { ctx });
    let step = runner.step().await;
    let state: (String, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT state,lease_token FROM jobs WHERE id=$1")
            .bind(previous.job_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let attempts = extraction::load_attempts(&db.pool, previous.job_id)
        .await
        .unwrap();
    db.drop().await;
    assert_eq!(
        step.unwrap(),
        StepOutcome::CancellationPending {
            job_id: previous.job_id
        }
    );
    assert_eq!(state.0, "cancel_requested");
    assert!(state.1.is_some_and(|token| token != previous.token));
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].request, request);
}

#[tokio::test]
async fn t48_real_office_scan_capture_http_publication_and_failed_budget() {
    let db = TestDb::create().await.unwrap();
    let root = std::env::temp_dir().join(format!("t48-pipeline-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let staging = root.join("staging");
    std::fs::create_dir_all(&source).unwrap();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
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
    assert!(
        Command::new(&python)
            .args(["-I", "-B"])
            .arg(repository.join("crates/knoted-core/tests/fixtures/t48_office.py"))
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(source.join("corrupt.docx"), b"broken zip").unwrap();
    std::fs::copy(source.join("sample.xlsx"), source.join("mislabeled.docx")).unwrap();
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
        WorkerRunner::with_context(ctx.clone(), "t48-pipeline", Dispatcher { ctx: ctx.clone() });
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
                    matches!(
                        outcome,
                        jobs::ExecutionOutcome::Succeeded
                            | jobs::ExecutionOutcome::PermanentFailure { .. }
                    ),
                    "{}: {outcome:?}",
                    job.kind
                ),
                other => panic!("{other:?}"),
            }
        }
        let rows:Vec<(uuid::Uuid,String,uuid::Uuid,String)>=sqlx::query_as("SELECT execution_id,blocks::text,job_id,parser_revision FROM file_extractions ORDER BY id").fetch_all(&db.pool).await.unwrap();
        assert_eq!(rows.len(), 3);
        for (id, blocks, job_id, revision) in rows {
            assert!(blocks.contains("中文"), "{blocks}");
            assert!(knoted_core::files::office::format_for_parser(&revision).is_some());
            let attempts = extraction::load_attempts(&db.pool, job_id).await.unwrap();
            assert_eq!(attempts.len(), 1);
            assert_eq!(id, attempts[0].request.execution_id);
            assert_eq!(
                attempts[0].request.options_revision,
                knoted_core::files::office::OPTIONS_REVISION
            );
            client
                .get_execution(id)
                .await
                .unwrap()
                .verify_resource_exit(&attempts[0].request)
                .unwrap();
        }
        let locations:Vec<(String,Option<i64>,String)>=sqlx::query_as("SELECT s.relative_path,(e.blocks->0->>'page')::bigint,e.blocks->0->>'structure' FROM file_extractions e JOIN file_sources s ON s.id=e.source_id ORDER BY s.relative_path").fetch_all(&db.pool).await.unwrap();
        assert_eq!(locations[0].0, "sample.docx");
        assert_eq!(locations[0].1, None);
        assert_eq!(locations[1].0, "sample.pptx");
        assert_eq!(locations[1].1, Some(2));
        assert_eq!(locations[2].0, "sample.xlsx");
        assert_eq!(locations[2].1, None);
        assert!(locations[2].2.contains("中文'原名") && locations[2].2.contains("C3"));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(
            count, 5,
            "scan {scan} cannot reopen unchanged success or failed input"
        );
        let failed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE kind='file.extract' AND state='failed'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(failed, 2);
        // A metadata-only edit must not replace the Office content identity.
        sqlx::query("UPDATE file_sources SET note='metadata only',version=version+1")
            .execute(&db.pool)
            .await
            .unwrap();
    }
    // The exact same inode/content recovers under its corrected format identity.
    let old: (uuid::Uuid, i64) = sqlx::query_as(
        "SELECT id,content_version FROM file_sources WHERE relative_path='mislabeled.docx'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let source = ctx.files().root.as_ref().unwrap();
    std::fs::rename(
        source.join("mislabeled.docx"),
        source.join("corrected.xlsx"),
    )
    .unwrap();
    scan_once(&ctx, &runner).await;
    let corrected:(uuid::Uuid,i64)=sqlx::query_as("SELECT id,content_version FROM file_sources WHERE relative_path='corrected.xlsx' AND status='present'").fetch_one(&db.pool).await.unwrap();
    assert_eq!(corrected, old);
    let revisions: Vec<String> =
        sqlx::query_scalar("SELECT parser_revision FROM file_extractions WHERE source_id=$1")
            .bind(old.0)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        revisions,
        vec![knoted_core::files::office::XLSX_PARSER_REVISION]
    );
    let successful: (uuid::Uuid, i64) = sqlx::query_as(
        "SELECT id,content_version FROM file_sources WHERE relative_path='sample.docx'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    std::fs::rename(source.join("sample.docx"), source.join("changed.xlsx")).unwrap();
    scan_once(&ctx, &runner).await;
    let renamed:(uuid::Uuid,i64)=sqlx::query_as("SELECT id,content_version FROM file_sources WHERE relative_path='changed.xlsx' AND status='present'").fetch_one(&db.pool).await.unwrap();
    assert_eq!(renamed, successful);
    let attempts:Vec<(String,String)>=sqlx::query_as("SELECT payload->>'parser_revision',state FROM jobs WHERE kind='file.extract' AND payload->>'source_id'=$1 ORDER BY created_at").bind(successful.0.to_string()).fetch_all(&db.pool).await.unwrap();
    assert_eq!(
        attempts,
        vec![
            (
                knoted_core::files::office::DOCX_PARSER_REVISION.into(),
                "succeeded".into()
            ),
            (
                knoted_core::files::office::XLSX_PARSER_REVISION.into(),
                "failed".into()
            )
        ]
    );
    // An upgrade can inherit a durable attempt from the old DOCX parser.
    // Use an existing closed input and the real loopback supervisor to prove
    // the replacement worker settles it without launching another execution.
    let previous_job: uuid::Uuid = sqlx::query_scalar(
        "SELECT job_id FROM file_extractions WHERE source_id=$1 AND parser_revision=$2",
    )
    .bind(successful.0)
    .bind(office::DOCX_PARSER_REVISION)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let payload = sqlx::query_scalar(
        "SELECT jsonb_set(payload-'intelligence_attempts','{parser_revision}',to_jsonb($2::text))
         FROM jobs WHERE id=$1",
    )
    .bind(previous_job)
    .bind("office-ooxml-body-v2-hidden-slides-included-cache-only:docx")
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let mut spec = jobs::JobSpec::new(extraction::EXTRACT_JOB_KIND);
    spec.payload = payload;
    let mut tx = db.pool.begin().await.unwrap();
    let retired = jobs::enqueue(&mut tx, spec).await.unwrap().job;
    tx.commit().await.unwrap();
    let old_lease = jobs::claim(&db.pool, "old-docx").await.unwrap().unwrap();
    assert_eq!(old_lease.job_id, retired.id);
    let previous = extraction::load_attempts(&db.pool, previous_job)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        deadline: ctx.now() + Duration::from_secs(900),
        ..previous.request
    };
    extraction::register_attempt(&db.pool, &old_lease, request.clone())
        .await
        .unwrap();
    client.create_execution(&request).await.unwrap();
    sqlx::query(
        "UPDATE jobs SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(retired.id)
    .execute(&db.pool)
    .await
    .unwrap();
    let settled = runner.step().await.unwrap();
    assert!(
        matches!(
            settled,
            StepOutcome::Finished { ref job, outcome: jobs::ExecutionOutcome::Cancelled }
                if job.id == retired.id && job.state == "cancelled"
        ),
        "{settled:?}"
    );
    client
        .get_execution(request.execution_id)
        .await
        .unwrap()
        .verify_resource_exit(&request)
        .unwrap();
    let history = extraction::load_attempts(&db.pool, retired.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].request, request);
    let published: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM file_extractions WHERE job_id=$1)")
            .bind(retired.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(!published);
    drop(service);
    db.drop().await;
}
