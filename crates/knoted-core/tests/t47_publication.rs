use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    db::testing::TestDb,
    files::{
        capture::{SourceObservation, capture_stable},
        extraction::{self, ExtractionInput},
        safe_fs,
        scanner::{ObserveRoot, observe_root, reconcile_complete_scan},
    },
};
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;
async fn lease(ctx: &AppContext) -> knoted_core::jobs::Lease {
    let mut tx = ctx.pool().begin().await.unwrap();
    knoted_core::jobs::enqueue(&mut tx, knoted_core::jobs::JobSpec::new("test.capture"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    knoted_core::jobs::claim(ctx.pool(), "t44")
        .await
        .unwrap()
        .unwrap()
}
async fn source_fixture(size: usize) -> (TestDb, AppContext, TempDir, TempDir, SourceObservation) {
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.pdf"), vec![b'a'; size]).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.path().into()),
        staging_root: Some(staging.path().into()),
        ..Default::default()
    });
    let l = lease(&ctx).await;
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: l.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let scan_id = report.scan_id;
    let root_id = report.root_id;
    let generation = report.generation;
    assert!(reconcile_complete_scan(&ctx, report).await.unwrap());
    let fd = safe_fs::open_root(root.path()).unwrap();
    let file = std::fs::File::open(root.path().join("a.pdf")).unwrap();
    let input = SourceObservation {
        scan_id,
        root_id,
        generation,
        root_path: root.path().into(),
        root_identity: safe_fs::metadata(fd).unwrap().identity,
        relative_path: PathBuf::from("a.pdf"),
        metadata: safe_fs::metadata(file).unwrap(),
        lease: l,
        cancel: Arc::default(),
    };
    (db, ctx, root, staging, input)
}
async fn extraction_fixture() -> (
    TestDb,
    AppContext,
    TempDir,
    TempDir,
    knoted_core::jobs::Lease,
    ExtractionInput,
) {
    let (db, ctx, root, staging, observation) = source_fixture(3).await;
    capture_stable(&ctx, observation).await.unwrap();
    sqlx::query("UPDATE jobs SET state='cancelled',lease_token=NULL,lease_expires_at=NULL,locked_by=NULL WHERE kind<>'file.extract'").execute(ctx.pool()).await.unwrap();
    let lease = knoted_core::jobs::claim(ctx.pool(), "t46")
        .await
        .unwrap()
        .unwrap();
    let job = knoted_core::jobs::load_claimed(ctx.pool(), &lease)
        .await
        .unwrap()
        .unwrap();
    let input = ExtractionInput::from_job(&job).unwrap();
    (db, ctx, root, staging, lease, input)
}

#[tokio::test]
async fn t47_pdf_enqueues_correct_identity_and_publishes_real_execution() {
    use knoted_core::{
        files::pdf,
        intelligence::{ExecutionKind, ExecutionRequest},
    };
    let (db, ctx, _root, _staging, lease, input) = extraction_fixture().await;
    let job = knoted_core::jobs::load_claimed(ctx.pool(), &lease)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.payload["parser_revision"], pdf::PARSER_REVISION);
    let request = ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: input.staged_object().unwrap().object_id,
        input_sha256: input.staged_object().unwrap().sha256,
        source_version: input.content_version,
        deadline: chrono::Utc::now() + chrono::Duration::seconds(900),
        options_revision: pdf::OPTIONS_REVISION.into(),
    };
    let attempt = extraction::register_attempt(ctx.pool(), &lease, request)
        .await
        .unwrap();
    let doc = knoted_core::files::text::ExtractedDocument {
        blocks: vec![],
        title: None,
        source_format: "pdf".into(),
        page_count: Some(1),
        truncated: false,
    };
    extraction::publish_python(&ctx, &lease, &input, &doc, &attempt)
        .await
        .unwrap();
    let id: uuid::Uuid = sqlx::query_scalar("SELECT execution_id FROM file_extractions")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    assert_eq!(id, attempt.request.execution_id);
    sqlx::query("UPDATE file_sources SET observation_version=observation_version+1")
        .execute(ctx.pool())
        .await
        .unwrap();
    assert!(matches!(
        extraction::publish_python(&ctx, &lease, &input, &doc, &attempt).await,
        Err(extraction::DomainError::Stale)
    ));
    db.drop().await;
}

#[tokio::test]
async fn t47_python_deadline_crossed_waiting_for_publication_locks_rolls_back() {
    use knoted_core::{
        files::pdf,
        intelligence::{ExecutionKind, ExecutionRequest},
    };
    for table in ["file_roots", "file_sources"] {
        let (db, ctx, _root, _staging, lease, input) = extraction_fixture().await;
        let request = ExecutionRequest {
            execution_id: uuid::Uuid::new_v4(),
            kind: ExecutionKind::Parse,
            input_object_id: input.staged_object().unwrap().object_id,
            input_sha256: input.staged_object().unwrap().sha256,
            source_version: input.content_version,
            deadline: chrono::Utc::now() + chrono::Duration::seconds(2),
            options_revision: pdf::OPTIONS_REVISION.into(),
        };
        let attempt = extraction::register_attempt(ctx.pool(), &lease, request)
            .await
            .unwrap();
        let deadline = attempt.request.deadline;
        let mut blocker = db.pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let (sql, id) = if table == "file_roots" {
            (
                "SELECT id FROM file_roots WHERE id=$1 FOR UPDATE",
                input.root_id,
            )
        } else {
            (
                "SELECT id FROM file_sources WHERE id=$1 FOR UPDATE",
                input.source_id,
            )
        };
        sqlx::query(sql)
            .bind(id)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let publisher = tokio::spawn(async move {
            let doc = knoted_core::files::text::ExtractedDocument {
                blocks: vec![],
                title: None,
                source_format: "pdf".into(),
                page_count: Some(1),
                truncated: false,
            };
            extraction::publish_python(&ctx, &lease, &input, &doc, &attempt).await
        });
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))")
                .bind(blocker_pid).fetch_one(&mut *blocker).await.unwrap();
            if blocked {
                break;
            }
            assert!(
                chrono::Utc::now() < deadline,
                "publisher never reached {table} lock before deadline"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(chrono::Utc::now() < deadline);
        tokio::time::sleep(
            (deadline - chrono::Utc::now()).to_std().unwrap()
                + std::time::Duration::from_millis(30),
        )
        .await;
        blocker.commit().await.unwrap();
        let result = publisher.await.unwrap();
        assert!(
            matches!(result, Err(extraction::DomainError::DeadlineExceeded)),
            "publication after {table} lock crossed its deadline: {result:?}"
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='file.extracted'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!((rows, events), (0, 0));
        db.drop().await;
    }
}
