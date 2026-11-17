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
    let runner = WorkerRunner::with_context(ctx.clone(), "t44", Dispatcher { ctx: ctx.clone() });
    let scan = runner.step().await.unwrap();
    let capture = runner.step().await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources WHERE content_fresh")
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
    assert_eq!(rows, 1);
}

fn temp() -> std::path::PathBuf {
    let p = std::path::PathBuf::from(format!(
        "/data/knoted-dev/tasks/t44/tmp/{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&p).unwrap();
    p
}
#[tokio::test]
async fn t44_real_runner_cancellation_stops_and_cleans_active_copy() {
    use std::time::Duration;
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    let file = std::fs::File::create(root.join("sample.bin")).unwrap();
    file.set_len(64 * 1024 * 1024).unwrap();
    drop(file);
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.clone()),
        staging_root: Some(staging.clone()),
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
            let writing = std::fs::read_dir(staging.join("input"))
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
    let objects = std::fs::read_dir(staging.join("input")).unwrap().count();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(result,StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert_eq!(sources, 0);
    assert_eq!(objects, 0);
}
