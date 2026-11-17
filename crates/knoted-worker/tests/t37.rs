//! Run the actual Worker process, including its startup and periodic trash wiring.
use knoted_core::{
    context::{AppContext, FixedClock},
    db::testing::{TestDb, test_database_url},
    tasks::service::{self, CreateTask, DeleteTask},
};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Worker {
    child: Child,
    config: PathBuf,
    log: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.config);
        let _ = fs::remove_file(&self.log);
    }
}
fn start(db: &TestDb) -> Worker {
    let base = test_database_url();
    let url = format!(
        "{base}{}options=-csearch_path%3D{}%2Cpublic",
        if base.contains('?') { "&" } else { "?" },
        db.schema()
    );
    let config = std::env::temp_dir().join(format!("knoted-t37-{}.toml", Uuid::new_v4()));
    fs::write(&config, format!("[database]\nurl = {url:?}\n")).unwrap();
    let log = config.with_extension("log");
    let child = Command::new(env!("CARGO_BIN_EXE_knoted-worker"))
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    Worker { child, config, log }
}
async fn expired(ctx: &AppContext) -> Uuid {
    let t = service::create(
        ctx,
        CreateTask {
            title: "expired".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::delete(
        ctx,
        DeleteTask {
            id: t.id,
            expected_version: 1,
        },
    )
    .await
    .unwrap();
    t.id
}
async fn wait_for_purge(db: &TestDb, worker: &mut Worker, id: Uuid, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        assert!(worker.child.try_wait().unwrap().is_none(), "worker exited");
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=$1)")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        if !exists {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not purge eligible task"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// Detects startup-only or unwired maintenance; the normal history loop must stay alive too.
#[tokio::test]
async fn t37_worker_purges_on_startup_and_each_maintenance_round() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let past = ctx.now() - Duration::from_secs(31 * 86_400);
    let ctx = ctx.with_clock(Arc::new(FixedClock::new(past)));
    let first = expired(&ctx).await;
    let mut worker = start(&db);
    wait_for_purge(&db, &mut worker, first, 10).await;
    let second = expired(&ctx).await;
    wait_for_purge(&db, &mut worker, second, 40).await;
    let identities: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_registry WHERE id=ANY($1)")
            .bind(vec![first, second])
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(identities, 2);
    let log = fs::read_to_string(&worker.log).unwrap();
    assert!(log.contains("task trash pass purged=1"));
    assert!(
        log.contains("note history pass"),
        "history maintenance must remain independent"
    );
}

// A DB failure must neither kill the worker nor leak DB diagnostics/content.
#[tokio::test]
async fn t37_worker_retries_failures_without_logging_task_or_database_content() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let past = ctx.now() - Duration::from_secs(31 * 86_400);
    let ctx = ctx.with_clock(Arc::new(FixedClock::new(past)));
    let id = expired(&ctx).await;
    sqlx::raw_sql("CREATE FUNCTION reject_trash() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private-task-body-marker'; END $$; CREATE TRIGGER reject_trash BEFORE DELETE ON tasks FOR EACH ROW EXECUTE FUNCTION reject_trash()").execute(&db.pool).await.unwrap();
    let mut worker = start(&db);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(worker.child.try_wait().unwrap().is_none());
        let log = fs::read_to_string(&worker.log).unwrap();
        assert!(!log.contains("private-task-body-marker"));
        if log.contains("task trash maintenance unavailable") && log.contains("note history pass") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "missing sanitized failure or independent history pass"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    sqlx::query("DROP TRIGGER reject_trash ON tasks")
        .execute(&db.pool)
        .await
        .unwrap();
    wait_for_purge(&db, &mut worker, id, 40).await;
    let log = fs::read_to_string(&worker.log).unwrap();
    assert!(log.contains("task trash pass purged=1"));
    assert!(!log.contains("private-task-body-marker"));
}
