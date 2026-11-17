use knoted_core::{
    config::FilesConfig, context::AppContext, db::testing::TestDb, files::scanner::enqueue_scan,
};
use knoted_worker::{
    handlers::scan::Dispatcher,
    runner::{StepOutcome, WorkerRunner},
};
#[tokio::test]
async fn t43_worker_runs_queued_scan_through_real_lease_and_preserves_busy_event() {
    let db = TestDb::create().await.unwrap();
    let path = std::path::PathBuf::from(format!(
        "/data/knoted-dev/tasks/t43/tmp/worker-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("sample.txt"), "sample").unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(path.clone()),
        staging_root: Some("/private/staging".into()),
        ..Default::default()
    });
    assert!(enqueue_scan(&ctx).await.unwrap());
    assert!(!enqueue_scan(&ctx).await.unwrap());
    let runner = WorkerRunner::with_context(ctx.clone(), "t43", Dispatcher { ctx: ctx.clone() });
    let StepOutcome::Finished { job, .. } = runner.step().await.unwrap() else {
        panic!("job must finish")
    };
    assert_eq!(job.state, "succeeded");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_scans WHERE complete")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let paths: Vec<String> = sqlx::query_scalar("SELECT relative_path FROM file_observations")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    assert_eq!(paths, vec!["sample.txt"]);
    assert!(
        enqueue_scan(&ctx).await.unwrap(),
        "event retained while busy must be able to enqueue a fresh scan after completion"
    );
    std::fs::remove_dir_all(path).unwrap();
    db.drop().await;
}

#[tokio::test]
async fn t43_runtime_retains_real_notifications_while_scan_is_active() {
    use std::time::Duration;
    let db = TestDb::create().await.unwrap();
    let path = std::path::PathBuf::from(format!(
        "/data/knoted-dev/tasks/t43/tmp/runtime-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&path).unwrap();
    let parent = path;
    let path = parent.join("source");
    std::fs::create_dir(&path).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(path.clone()),
        staging_root: Some("/private/staging".into()),
        rescan_seconds: 60,
        ..Default::default()
    });
    let task = tokio::spawn(knoted_worker::handlers::scan::run(ctx.clone()));
    async fn completed(ctx: &AppContext, want: i64) -> bool {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let n: i64 = sqlx::query_scalar("SELECT count(*) FROM file_scans WHERE complete")
                    .fetch_one(ctx.pool())
                    .await
                    .unwrap();
                if n >= want {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok()
    }
    let first = completed(&ctx, 1).await;
    // Recursive watches are installed after registration; no source polling is
    // used. Keep the next scan active behind the root lock while another edit
    // arrives; that notification must survive completion of the first job.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut lock = ctx.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM file_roots FOR UPDATE")
        .execute(&mut *lock)
        .await
        .unwrap();
    std::fs::write(path.join("a.txt"), "one").unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    std::fs::write(path.join("b.txt"), "during active scan").unwrap();
    lock.commit().await.unwrap();
    let followup = completed(&ctx, 3).await;
    let paths:Vec<String>=sqlx::query_scalar("SELECT relative_path FROM file_observations WHERE scan_id=(SELECT id FROM file_scans WHERE complete ORDER BY generation DESC LIMIT 1) ORDER BY relative_path").fetch_all(ctx.pool()).await.unwrap();
    task.abort();
    let _ = task.await;
    std::fs::remove_dir_all(parent).unwrap();
    db.drop().await;
    assert!(first, "startup must enqueue a scan");
    assert!(
        followup,
        "events during active scan must cause a subsequent scan"
    );
    assert_eq!(paths, vec!["a.txt", "b.txt"]);
}
