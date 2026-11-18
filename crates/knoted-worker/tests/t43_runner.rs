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
    let path = std::env::temp_dir().join(format!("worker-{}", uuid::Uuid::new_v4()));
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
    let path = std::env::temp_dir().join(format!("runtime-{}", uuid::Uuid::new_v4()));
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

async fn concurrent_pool(db: &TestDb) -> sqlx::PgPool {
    let schema = db.schema().to_owned();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |connection, _| {
            let query = format!("SET search_path TO {schema}, public");
            Box::pin(async move {
                sqlx::query(&query).execute(connection).await?;
                Ok(())
            })
        })
        .connect_with((*db.pool.connect_options()).clone())
        .await
        .unwrap()
}
#[tokio::test]
async fn t43_r1_heartbeat_wait_keeps_polling_scan_holding_job_lock() {
    use std::time::Duration;
    let db = TestDb::create().await.unwrap();
    let pool = concurrent_pool(&db).await;
    let path = std::env::temp_dir().join(format!("r1-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    let ctx = AppContext::new(pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(path.clone()),
        staging_root: Some("/private/staging".into()),
        ..Default::default()
    });
    sqlx::query("INSERT INTO file_roots(id,configured_path) VALUES($1,$2)")
        .bind(uuid::Uuid::new_v4())
        .bind(path.to_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    enqueue_scan(&ctx).await.unwrap();
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM file_roots FOR UPDATE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let runner = WorkerRunner::with_context(ctx.clone(), "r1", Dispatcher { ctx: ctx.clone() })
        .with_heartbeat_interval(Duration::from_millis(20));
    let execution = tokio::time::timeout(Duration::from_secs(2), runner.step());
    let release = async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        lock.commit().await.unwrap();
    };
    let (result, ()) = tokio::join!(execution, release);
    pool.close().await;
    std::fs::remove_dir_all(path).unwrap();
    db.drop().await;
    assert!(
        matches!(result,Ok(Ok(StepOutcome::Finished {ref job,..})) if job.state=="succeeded"),
        "heartbeat must not stop polling the transaction that releases its row lock: {result:?}"
    );
}

#[tokio::test]
async fn t43_r3_runner_cancel_drops_observed_report_without_losing_invalidation() {
    use knoted_core::{
        files::scanner::{self, ObserveRoot},
        jobs::{self, ExecutionOutcome, Lease},
    };
    use knoted_worker::runner::Handler;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    struct ObservedThenWait {
        ctx: AppContext,
        observed: Arc<tokio::sync::Notify>,
        dropped: Arc<AtomicBool>,
    }
    struct DropSignal(Arc<AtomicBool>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    impl Handler for ObservedThenWait {
        async fn execute<'a>(&'a self, lease: &'a Lease) -> ExecutionOutcome {
            let _drop = DropSignal(self.dropped.clone());
            let _report = scanner::observe_root(
                &self.ctx,
                ObserveRoot {
                    path: self.ctx.files().root.clone().unwrap(),
                    cancel: Arc::default(),
                    lease: lease.clone(),
                    rules: Default::default(),
                },
            )
            .await
            .unwrap();
            // Hold the actual observed report before reconciliation. The real
            // runner must drop this future when its heartbeat sees cancellation;
            // no test calls reconcile_complete_scan to flush on its behalf.
            self.observed.notify_one();
            std::future::pending().await
        }
    }
    let db = TestDb::create().await.unwrap();
    let pool = concurrent_pool(&db).await;
    let path = std::env::temp_dir().join(format!("r3-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("a.txt"), "old").unwrap();
    let ctx = AppContext::new(pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(path.clone()),
        staging_root: Some("/private/staging".into()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    let initial =
        WorkerRunner::with_context(ctx.clone(), "initial", Dispatcher { ctx: ctx.clone() });
    initial.step().await.unwrap();
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    // Stable metadata from the successful baseline scan; digest is SHA256('old').
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,content_fresh,last_seen_at) SELECT $1,s.root_id,o.relative_path,(o.metadata->'identity'->>'device')::bigint,(o.metadata->'identity'->>'inode')::bigint,(o.metadata->>'size')::bigint,(o.metadata->>'mtime_ns')::bigint,(o.metadata->>'ctime_ns')::bigint,(o.metadata->>'nlink')::bigint,'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4',1,1,'present',o.metadata,true,now() FROM file_scans s JOIN file_observations o ON s.id=o.scan_id WHERE o.relative_path='a.txt'").bind(id).execute(&pool).await.unwrap();
    std::fs::write(path.join("a.txt"), "changed").unwrap();
    enqueue_scan(&ctx).await.unwrap();
    let observed = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let runner = WorkerRunner::with_context(
        ctx.clone(),
        "cancel",
        ObservedThenWait {
            ctx: ctx.clone(),
            observed: observed.clone(),
            dropped: dropped.clone(),
        },
    )
    .with_heartbeat_interval(Duration::from_millis(5));
    let task = tokio::spawn(async move { runner.step().await });
    let observed_ok = tokio::time::timeout(Duration::from_secs(2), observed.notified())
        .await
        .is_ok();
    let job: uuid::Uuid = sqlx::query_scalar("SELECT id FROM jobs WHERE state='running'")
        .fetch_one(&pool)
        .await
        .unwrap();
    jobs::service::request_cancel(&ctx, job).await.unwrap();
    let mut task = task;
    let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
    let cancelled =
        matches!(&result,Ok(Ok(Ok(StepOutcome::Finished {job,..}))) if job.state=="cancelled");
    if result.is_err() {
        task.abort();
        let _ = task.await;
    }
    let state: (String, i64, i64, bool) = sqlx::query_as(
        "SELECT status,version,observation_version,content_fresh FROM file_sources WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let complete: bool =
        sqlx::query_scalar("SELECT complete FROM file_scans ORDER BY generation DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    pool.close().await;
    std::fs::remove_dir_all(path).unwrap();
    db.drop().await;
    assert!(observed_ok);
    assert!(cancelled);
    assert!(
        dropped.load(Ordering::Acquire),
        "runner must drop the execute future"
    );
    assert_eq!(state, ("present".into(), 2, 2, false));
    assert!(!complete);
}
