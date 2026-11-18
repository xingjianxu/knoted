use knoted_core::{
    db::testing::TestDb,
    jobs::{self, ExecutionOutcome, Lease},
};
use knoted_worker::runner::{Handler, WorkerRunner};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
struct Running {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}
impl Handler for Running {
    async fn execute(&self, _: &Lease) -> ExecutionOutcome {
        self.started.store(true, Ordering::SeqCst);
        std::future::pending().await
    }
    async fn cancel_and_wait(&self, _: &Lease) -> bool {
        self.stopped.store(true, Ordering::SeqCst);
        true
    }
}
#[tokio::test]
async fn t47_shutdown_stops_claiming_and_waits_for_registered_work() {
    let db = TestDb::create().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    jobs::enqueue(&mut tx, jobs::JobSpec::new("test.running"))
        .await
        .unwrap();
    jobs::enqueue(&mut tx, jobs::JobSpec::new("test.queued"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let runner = WorkerRunner::new(
        db.pool.clone(),
        "t47-shutdown",
        Running {
            started: started.clone(),
            stopped: stopped.clone(),
        },
    );
    runner
        .run_until(async {
            while !started.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
    assert!(stopped.load(Ordering::SeqCst));
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE state='queued'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(queued, 1);
    db.drop().await;
}
