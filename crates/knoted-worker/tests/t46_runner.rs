use knoted_core::{
    config::FilesConfig, context::AppContext, db::testing::TestDb, files::scanner::enqueue_scan,
};
use knoted_worker::{
    handlers::scan::Dispatcher,
    runner::{StepOutcome, WorkerRunner},
};
#[tokio::test]
async fn t46_dispatcher_captures_after_scan_via_shared_queue() {
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    std::fs::write(root.as_path().join("sample.txt"), "sample").unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.as_path().into()),
        staging_root: Some(staging.as_path().into()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    let runner = WorkerRunner::with_context(ctx.clone(), "t46", Dispatcher { ctx: ctx.clone() });
    let scan = Box::pin(runner.step()).await.unwrap();
    let capture = Box::pin(runner.step()).await.unwrap();
    let extraction = Box::pin(runner.step()).await.unwrap();
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind='file.extract' AND state='succeeded'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(scan,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert!(
        matches!(capture,StepOutcome::Finished{job,..} if job.state=="succeeded"),
        "capture dispatcher must succeed"
    );
    assert!(
        matches!(extraction,StepOutcome::Finished{job,..} if job.state=="succeeded"),
        "real extraction must run"
    );
    assert_eq!(rows, 1);
}

fn temp() -> std::path::PathBuf {
    let p = std::path::PathBuf::from(format!(
        "/data/knoted-dev/tasks/t46/tmp/{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&p).unwrap();
    p
}

async fn setup(
    bytes: &[u8],
    name: &str,
) -> (TestDb, AppContext, std::path::PathBuf, std::path::PathBuf) {
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    std::fs::write(root.join(name), bytes).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.clone()),
        staging_root: Some(staging.clone()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    (db, ctx, root, staging)
}
#[tokio::test]
async fn t46_invalid_encoding_and_nul_are_permanent_empty_is_success() {
    for (bytes, expected) in [
        (&b""[..], "succeeded"),
        (&b"\xff"[..], "failed"),
        (&b"a\0b"[..], "failed"),
    ] {
        let (db, ctx, root, staging) = setup(bytes, "sample.txt").await;
        let runner = WorkerRunner::with_context(
            ctx.clone(),
            "t46-terminal",
            Dispatcher { ctx: ctx.clone() },
        );
        Box::pin(runner.step()).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        let outcome = Box::pin(runner.step()).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
        db.drop().await;
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(staging).unwrap();
        assert!(
            matches!(outcome,StepOutcome::Finished{job,..} if job.state==expected && job.attempts==1)
        );
        assert_eq!(rows, if expected == "succeeded" { 1 } else { 0 });
    }
}
#[tokio::test]
async fn t46_existing_capture_backfills_and_metadata_replay_keeps_success() {
    let (db, ctx, root, staging) = setup(b"existing", "README").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-existing", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    // Model a pre-T46 closed source: remove only its never-started extraction job.
    sqlx::query("DELETE FROM jobs WHERE kind='file.extract'")
        .execute(ctx.pool())
        .await
        .unwrap();
    enqueue_scan(&ctx).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let result = Box::pin(runner.step()).await.unwrap();
    let before: (uuid::Uuid, i64) =
        sqlx::query_as("SELECT id,content_version FROM file_extractions")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    sqlx::query("UPDATE file_sources SET version=version+1,note='metadata only'")
        .execute(ctx.pool())
        .await
        .unwrap();
    enqueue_scan(&ctx).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let after: Vec<(uuid::Uuid, i64)> =
        sqlx::query_as("SELECT id,content_version FROM file_extractions")
            .fetch_all(ctx.pool())
            .await
            .unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(result,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert_eq!(after, vec![before]);
    assert_eq!(jobs, 1);
}
#[tokio::test]
async fn t46_superseded_queued_attempt_does_not_block_new_observation() {
    let (db, ctx, root, staging) = setup(b"old", "sample.txt").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-stale", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    std::fs::write(root.join("sample.txt"), b"new body").unwrap();
    enqueue_scan(&ctx).await.unwrap();
    sqlx::query("UPDATE jobs SET priority=10 WHERE kind='file.scan' AND state='queued'")
        .execute(ctx.pool())
        .await
        .unwrap();
    Box::pin(runner.step()).await.unwrap();
    let old = Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let new = Box::pin(runner.step()).await.unwrap();
    let bodies: Vec<String> = sqlx::query_scalar("SELECT blocks->0->>'text' FROM file_extractions")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(old,StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert!(matches!(new,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert_eq!(bodies, vec!["new body"]);
}
#[tokio::test]
async fn t46_later_binary_formats_never_become_text_success() {
    let (db, ctx, root, staging) = setup(b"plain bytes named pdf", "sample.PDF").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-format", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn t46_running_text_cancellation_publishes_nothing() {
    let bytes = vec![b'x'; 100 * 1024 * 1024];
    let (db, ctx, root, staging) = setup(&bytes, "large.txt").await;
    drop(bytes);
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-cancel", Dispatcher { ctx: ctx.clone() })
            .with_heartbeat_interval(std::time::Duration::from_millis(1));
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let cancel = async {
        let id = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let id: Option<uuid::Uuid> = sqlx::query_scalar(
                    "SELECT id FROM jobs WHERE kind='file.extract' AND state='running'",
                )
                .fetch_optional(ctx.pool())
                .await
                .unwrap();
                if let Some(id) = id {
                    break id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        knoted_core::jobs::service::request_cancel(&ctx, id)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(Box::pin(runner.step()), cancel);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(result.unwrap(),StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert_eq!(count, 0);
}
