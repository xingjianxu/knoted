//! T07 acceptance test for the worker runner skeleton.
//!
//! The test uses the real local PostgreSQL test database through
//! [`TestDb`], so the claim → heartbeat → finish path is exercised against the
//! same lease primitives the production loop uses.

use std::time::Duration;

use knoted_core::db::testing::TestDb;
use knoted_core::jobs::{self, JobOutcome, JobSpec, Lease};
use knoted_worker::runner::{RunnerError, WorkerRunner};
use sqlx::PgPool;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

async fn enqueue(pool: &PgPool) -> uuid::Uuid {
    let mut tx = pool.begin().await.expect("a transaction must start");
    let job = jobs::enqueue(
        &mut tx,
        JobSpec {
            kind: "parse".to_owned(),
            ..JobSpec::default()
        },
    )
    .await
    .expect("enqueue");
    tx.commit()
        .await
        .expect("the enqueue transaction must commit");
    job.job.id
}

#[tokio::test]
async fn t07_runner_finishes_claimed_job() {
    let db = test_db().await;
    let job_id = enqueue(&db.pool).await;

    let runner = WorkerRunner::new(db.pool.clone(), "worker-a", |_lease: &Lease| async {
        JobOutcome::Succeeded
    });

    let outcome = runner.run_once().await.expect("the runner must not fail");
    assert_eq!(outcome, Some(JobOutcome::Succeeded));

    let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&db.pool)
        .await
        .expect("the job row must be readable");
    assert_eq!(state, "succeeded");

    let lease_released: bool = sqlx::query_scalar(
        "SELECT lease_token IS NULL AND locked_by IS NULL FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(&db.pool)
    .await
    .expect("the job row must be readable");
    assert!(lease_released, "finishing must release the lease");

    // The queue is empty now: the loop backs off instead of blocking.
    assert_eq!(
        runner
            .run_once()
            .await
            .expect("an empty queue must not fail"),
        None
    );

    db.drop().await;
}

#[tokio::test]
async fn t07_runner_stops_when_the_lease_is_lost() {
    let db = test_db().await;
    let job_id = enqueue(&db.pool).await;

    // The handler stalls while another worker takes the lease over; the
    // runner's heartbeat must notice and abandon the result instead of
    // reporting success for a job it no longer owns.
    let steal_pool = db.pool.clone();
    let runner = WorkerRunner::new(db.pool.clone(), "worker-a", move |lease: &Lease| {
        let pool = steal_pool.clone();
        let job_id = lease.job_id;
        async move {
            sqlx::query(
                "UPDATE jobs SET lease_token = gen_random_uuid(), locked_by = 'worker-b', \
                 lease_expires_at = now() + interval '90 seconds' WHERE id = $1",
            )
            .bind(job_id)
            .execute(&pool)
            .await
            .expect("the lease must be stealable in this test");
            tokio::time::sleep(Duration::from_millis(80)).await;
            JobOutcome::Succeeded
        }
    })
    .with_heartbeat_interval(Duration::from_millis(10));

    let error = runner
        .run_once()
        .await
        .expect_err("a lost lease must not report completion");
    assert!(
        matches!(error, RunnerError::LeaseLost { .. }),
        "unexpected error: {error:?}"
    );

    let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&db.pool)
        .await
        .expect("the job row must be readable");
    let locked_by: Option<String> = sqlx::query_scalar("SELECT locked_by FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&db.pool)
        .await
        .expect("the job row must be readable");
    assert_eq!(state, "running", "the stolen job must stay running");
    assert_eq!(
        locked_by.as_deref(),
        Some("worker-b"),
        "the stale worker must not clear the new lease"
    );

    db.drop().await;
}

#[tokio::test]
async fn t68_maintenance_leaves_queued_job_unclaimed() {
    let db = test_db().await;
    enqueue(&db.pool).await;
    let ctx = knoted_core::context::AppContext::new(db.pool.clone());
    let guard = knoted_core::maintenance::exclusive(ctx.admission_pool(), Duration::from_secs(2))
        .await
        .unwrap();
    let runner = WorkerRunner::new(db.pool.clone(), "paused", |_lease: &Lease| async {
        JobOutcome::Succeeded
    });
    assert_eq!(runner.run_once().await.unwrap(), None);
    drop(guard);
    assert_eq!(
        runner.run_once().await.unwrap(),
        Some(JobOutcome::Succeeded)
    );
    db.drop().await;
}
