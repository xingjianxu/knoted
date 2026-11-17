use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    db::testing::TestDb,
    files::{
        capture::{SourceObservation, capture_stable},
        extraction::{self, ExtractionInput},
        safe_fs,
        scanner::{ObserveRoot, observe_root, reconcile_complete_scan},
        text::extract_text,
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
    std::fs::write(root.path().join("a.txt"), vec![b'a'; size]).unwrap();
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
    let file = std::fs::File::open(root.path().join("a.txt")).unwrap();
    let input = SourceObservation {
        scan_id,
        root_id,
        generation,
        root_path: root.path().into(),
        root_identity: safe_fs::metadata(fd).unwrap().identity,
        relative_path: PathBuf::from("a.txt"),
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
async fn t46_publication_is_unique_metadata_independent_and_no_python_id() {
    let (db, ctx, _root, _staging, lease, input) = extraction_fixture().await;
    let doc = extract_text(&ctx, input.staged_object().unwrap())
        .await
        .unwrap();
    sqlx::query("UPDATE file_sources SET version=version+7,note='metadata changed'")
        .execute(ctx.pool())
        .await
        .unwrap();
    let first = extraction::publish(&ctx, &lease, &input, &doc).await;
    let second = extraction::publish(&ctx, &lease, &input, &doc).await;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM file_extractions WHERE content_version=1 AND execution_id IS NULL AND blocks->0->>'text'='aaa'").fetch_one(ctx.pool()).await.unwrap();
    let outbox: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='file.extracted'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok());
    assert_eq!(count, 1);
    assert_eq!(outbox, 1);
}
#[tokio::test]
async fn t46_publication_rejects_every_stale_fence() {
    for update in [
        "UPDATE jobs SET lease_expires_at=now()-interval '1 second' WHERE kind='file.extract'",
        "UPDATE jobs SET lease_token=gen_random_uuid() WHERE kind='file.extract'",
        "UPDATE jobs SET state='cancel_requested' WHERE kind='file.extract'",
        "UPDATE file_sources SET status='missing'",
        "UPDATE file_sources SET content_fresh=false",
        "UPDATE file_sources SET content_version=content_version+1",
        "UPDATE file_sources SET observation_version=observation_version+1",
        "UPDATE file_sources SET observation='{}'",
        "UPDATE file_sources SET sha256=repeat('0',64)",
        "UPDATE file_sources SET input_object_id='different'",
        "UPDATE file_roots SET paused_reason='unavailable'",
        "UPDATE file_roots SET scan_token=gen_random_uuid()",
    ] {
        let (db, ctx, _root, _staging, lease, input) = extraction_fixture().await;
        let doc = extract_text(&ctx, input.staged_object().unwrap())
            .await
            .unwrap();
        sqlx::query(update).execute(ctx.pool()).await.unwrap();
        let result = extraction::publish(&ctx, &lease, &input, &doc).await;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
        db.drop().await;
        assert!(
            matches!(result, Err(extraction::DomainError::Stale)),
            "{update}: {result:?}"
        );
        assert_eq!(count, 0);
    }
}
#[tokio::test]
async fn t46_unchanged_later_complete_scan_allows_publication() {
    let (db, ctx, root, _staging, lease, input) = extraction_fixture().await;
    let doc = extract_text(&ctx, input.staged_object().unwrap())
        .await
        .unwrap();
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    reconcile_complete_scan(&ctx, report).await.unwrap();
    let result = extraction::publish(&ctx, &lease, &input, &doc).await;
    db.drop().await;
    assert!(result.is_ok(), "{result:?}");
}
#[tokio::test]
async fn t46_expired_while_waiting_for_source_lock_rolls_back_result_and_outbox() {
    let (db, ctx, _root, _staging, lease, input) = extraction_fixture().await;
    let doc = extract_text(&ctx, input.staged_object().unwrap())
        .await
        .unwrap();
    let mut blocker = ctx.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM file_sources FOR UPDATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET lease_expires_at=clock_timestamp()+interval '150 milliseconds' WHERE id=$1").bind(lease.job_id).execute(ctx.pool()).await.unwrap();
    let release = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        blocker.rollback().await.unwrap();
    };
    let (result, ()) = tokio::join!(extraction::publish(&ctx, &lease, &input, &doc), release);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE topic='file.extracted'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(matches!(result, Err(extraction::DomainError::Stale)));
    assert_eq!((count, rows), (0, 0));
}
