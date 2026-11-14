//! T08 acceptance test for the worker runner's cancellation handshake.
//!
//! The runner must not release the execution slot until the handler confirms
//! that the cancelled child exited. The test drives that path against the real
//! PostgreSQL test database: the first worker reports "not exited yet" and keeps
//! the lease; a takeover after lease expiry confirms the exit and only then
//! releases it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use knoted_core::context::AppContext;
use knoted_core::db::testing::TestDb;
use knoted_core::jobs::service::{self, ExecutionOutcome};
use knoted_core::jobs::{self, JobSpec, Lease};
use knoted_worker::runner::{Handler, StepOutcome, WorkerRunner};
use sqlx::PgPool;
use tokio::sync::Notify;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

async fn enqueue(pool: &PgPool) -> uuid::Uuid {
    let mut tx = pool.begin().await.expect("a transaction must start");
    let job = jobs::enqueue(&mut tx, JobSpec::new("parse"))
        .await
        .expect("enqueue");
    tx.commit()
        .await
        .expect("the enqueue transaction must commit");
    job.job.id
}

async fn load_state(pool: &PgPool, id: uuid::Uuid) -> (String, Option<uuid::Uuid>) {
    sqlx::query_as("SELECT state, lease_token FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the job row must be readable")
}

/// A handler that blocks until cancelled and can pretend its child is still
/// running (`exit_confirmed = false`).
struct BlockingHandler {
    started: Arc<Notify>,
    exit_confirmed: bool,
    executed: Arc<AtomicUsize>,
}

impl Handler for BlockingHandler {
    async fn execute<'a>(&'a self, _lease: &'a Lease) -> ExecutionOutcome {
        self.executed.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        // The runner drops this future when it observes the cancellation;
        // sleeping keeps it alive until then without a busy loop.
        tokio::time::sleep(Duration::from_secs(30)).await;
        ExecutionOutcome::Succeeded
    }

    async fn cancel_and_wait<'a>(&'a self, _lease: &'a Lease) -> bool {
        self.exit_confirmed
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t08_runner_confirms_cancel_before_releasing_the_slot() {
    let db = test_db().await;
    let ctx = AppContext::new(db.pool.clone());
    let job_id = enqueue(&db.pool).await;

    let started = Arc::new(Notify::new());
    let executed = Arc::new(AtomicUsize::new(0));
    let handler = BlockingHandler {
        started: Arc::clone(&started),
        exit_confirmed: false,
        executed: Arc::clone(&executed),
    };
    let runner = WorkerRunner::with_context(ctx.clone(), "worker-a", handler)
        .with_heartbeat_interval(Duration::from_millis(10))
        .with_idle_backoff(Duration::from_millis(10));

    let step = tokio::spawn(async move { runner.step().await });
    started.notified().await;

    let requested = service::request_cancel(&ctx, job_id)
        .await
        .expect("the running job must be cancellable");
    assert_eq!(requested.state, "cancel_requested");

    let outcome = step
        .await
        .expect("the runner task must not panic")
        .expect("the runner must not fail");
    assert!(
        matches!(
            outcome,
            StepOutcome::CancellationPending { job_id: pending } if pending == job_id
        ),
        "an unconfirmed exit must be reported as pending, got {outcome:?}"
    );

    let (state, token) = load_state(&db.pool, job_id).await;
    assert_eq!(state, "cancel_requested");
    assert!(
        token.is_some(),
        "an unconfirmed exit must keep the lease and the execution slot"
    );
    assert_eq!(executed.load(Ordering::SeqCst), 1);

    // The worker that held the lease is gone (simulated by expiry); the
    // takeover repeats the confirmation instead of executing the work again.
    sqlx::query("UPDATE jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(job_id)
        .execute(&db.pool)
        .await
        .expect("the lease must be expirable");

    let takeover_executed = Arc::new(AtomicUsize::new(0));
    let takeover = WorkerRunner::with_context(
        ctx,
        "worker-b",
        BlockingHandler {
            started: Arc::new(Notify::new()),
            exit_confirmed: true,
            executed: Arc::clone(&takeover_executed),
        },
    )
    .with_heartbeat_interval(Duration::from_millis(10));

    let outcome = takeover.step().await.expect("the takeover must not fail");
    match outcome {
        StepOutcome::Finished { job, outcome } => {
            assert_eq!(outcome, ExecutionOutcome::Cancelled);
            assert_eq!(job.state, "cancelled");
            assert_eq!(job.lease_token, None, "confirmation releases the lease");
        }
        other => panic!("the confirmed cancellation must finish, got {other:?}"),
    }
    assert_eq!(
        takeover_executed.load(Ordering::SeqCst),
        0,
        "a takeover of a cancellation request must not execute the work again"
    );

    db.drop().await;
}

#[tokio::test]
async fn t08_runner_temporary_failure_enters_retry_wait() {
    let db = test_db().await;
    let ctx = AppContext::new(db.pool.clone());
    let job_id = enqueue(&db.pool).await;

    let runner = WorkerRunner::with_context(ctx, "worker-a", TemporaryFailureHandler);

    let outcome = runner.step().await.expect("the runner must not fail");
    match outcome {
        StepOutcome::Finished { job, outcome } => {
            assert_eq!(
                outcome,
                ExecutionOutcome::TemporaryFailure {
                    message: "upstream timed out".to_owned()
                }
            );
            assert_eq!(job.id, job_id);
            assert_eq!(job.state, "retry_wait");
            assert_eq!(job.attempts, 1);
            assert_eq!(job.lease_token, None);
        }
        other => panic!("a temporary failure must schedule a retry, got {other:?}"),
    }

    db.drop().await;
}

struct TemporaryFailureHandler;

impl Handler for TemporaryFailureHandler {
    async fn execute<'a>(&'a self, _lease: &'a Lease) -> ExecutionOutcome {
        ExecutionOutcome::TemporaryFailure {
            message: "upstream timed out".to_owned(),
        }
    }
}
