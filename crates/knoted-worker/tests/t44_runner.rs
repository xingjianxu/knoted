use knoted_core::{
    config::FilesConfig, context::AppContext, db::testing::TestDb, files::scanner::enqueue_scan,
};
use knoted_worker::{
    handlers::scan::Dispatcher,
    runner::{StepOutcome, WorkerRunner},
};
#[tokio::test]
async fn t44_dispatcher_captures_after_scan_via_shared_queue() {
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("sample.txt"), "sample").unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.path().into()),
        staging_root: Some(staging.path().into()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    let runner = WorkerRunner::with_context(ctx.clone(), "t44", Dispatcher { ctx: ctx.clone() });
    let scan = runner.step().await.unwrap();
    let capture = runner.step().await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources WHERE content_fresh")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(matches!(scan,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert!(
        matches!(capture,StepOutcome::Finished{job,..} if job.state=="succeeded"),
        "capture dispatcher must succeed"
    );
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn t44_real_runner_cancellation_stops_and_cleans_active_copy() {
    use std::time::Duration;
    let db = TestDb::create().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let file = std::fs::File::create(root.path().join("sample.bin")).unwrap();
    file.set_len(64 * 1024 * 1024).unwrap();
    drop(file);
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.path().into()),
        staging_root: Some(staging.path().into()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t44cancel", Dispatcher { ctx: ctx.clone() })
            .with_heartbeat_interval(Duration::from_millis(5));
    runner.step().await.unwrap();
    let task = tokio::spawn(async move { runner.step().await });
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let writing = std::fs::read_dir(staging.path().join("input"))
                .into_iter()
                .flatten()
                .any(|e| e.unwrap().path().extension().is_some_and(|v| v == "part"));
            if writing {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let job: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM jobs WHERE kind='file.capture' AND state='running'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    knoted_core::jobs::service::request_cancel(&ctx, job)
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let sources: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let objects = std::fs::read_dir(staging.path().join("input"))
        .unwrap()
        .count();
    db.drop().await;
    assert!(matches!(result,StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert_eq!(sources, 0);
    assert_eq!(objects, 0);
}
