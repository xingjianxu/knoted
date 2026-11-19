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
    let unresolved: i64 = sqlx::query_scalar("SELECT count(*) FROM maintenance_operations")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        unresolved, 0,
        "confirmed worker shutdown retires its active admission"
    );
    db.drop().await;
}

#[tokio::test]
async fn shutdown_attempts_every_owned_lease_and_keeps_unconfirmed_marker() {
    struct Unconfirmed(Arc<std::sync::atomic::AtomicUsize>);
    impl Handler for Unconfirmed {
        async fn execute(&self, _: &Lease) -> ExecutionOutcome {
            unreachable!()
        }
        async fn cancel_and_wait(&self, _: &Lease) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst) != 0
        }
    }

    let db = TestDb::create().await.unwrap();
    for kind in ["test.first", "test.second"] {
        let mut tx = db.pool.begin().await.unwrap();
        jobs::enqueue(&mut tx, jobs::JobSpec::new(kind))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        jobs::claim(&db.pool, "shutdown").await.unwrap().unwrap();
    }
    let visited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runner = WorkerRunner::new(db.pool.clone(), "shutdown", Unconfirmed(visited.clone()));
    runner.run_until(async {}).await;
    let unresolved: i64 = sqlx::query_scalar("SELECT count(*) FROM maintenance_operations")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(visited.load(Ordering::SeqCst), 2);
    assert_eq!(unresolved, 1);
    db.drop().await;
}
