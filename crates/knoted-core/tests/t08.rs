//! T08 acceptance tests: the retry/backoff and cancellation state machine.
//!
//! The tests run against the real local PostgreSQL test database
//! (`KNOTED_TEST_DATABASE_URL`, default
//! `postgres:///knoted_test?host=/run/postgresql`) with a per-test schema. A
//! missing database is a failure, never a skip.
//!
//! Every time-dependent decision goes through an injected [`FixedClock`], so
//! the suite never sleeps and asserts exact 5/30/120 second offsets instead of
//! observing real time.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use knoted_core::context::{AppContext, FixedClock};
use knoted_core::db::testing::TestDb;
use knoted_core::jobs::service::{self, DomainJobError, ExecutionOutcome, FinishExecution};
use knoted_core::jobs::{self, Job, JobSpec};
use sqlx::PgPool;
use uuid::Uuid;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

/// Automatic retry delays, in the order the attempt budget consumes them.
const BACKOFF_SECONDS: [i64; 3] = [5, 30, 120];

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

async fn enqueue(pool: &PgPool, max_attempts: i32) -> Job {
    let mut tx = pool.begin().await.expect("a transaction must start");
    let job = jobs::enqueue(
        &mut tx,
        JobSpec {
            kind: "parse".to_owned(),
            payload: serde_json::json!({ "input_object_id": "object-t08" }),
            max_attempts,
            ..JobSpec::default()
        },
    )
    .await
    .expect("enqueue");
    tx.commit()
        .await
        .expect("the enqueue transaction must commit");
    job.job
}

/// Builds a context whose clock is far enough in the future that a scheduled
/// retry is never due by accident: tests force `available_at` into the past when
/// they want the next claim to happen.
async fn test_context(pool: &PgPool) -> (AppContext, Arc<FixedClock>) {
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("the database clock must be readable");
    let clock = Arc::new(FixedClock::new(now + Duration::hours(1)));
    (
        AppContext::new(pool.clone()).with_clock(clock.clone()),
        clock,
    )
}

async fn claim(pool: &PgPool) -> knoted_core::jobs::Lease {
    jobs::claim(pool, "worker-a")
        .await
        .expect("claim must not fail")
        .expect("a due job must be claimable")
}

/// Claims due jobs until the one with `id` is handed out.
///
/// A test that enqueued helper jobs along the way must not depend on which of
/// them the claim order picks first.
async fn claim_job(pool: &PgPool, id: Uuid) -> knoted_core::jobs::Lease {
    loop {
        let lease = claim(pool).await;
        if lease.job_id == id {
            return lease;
        }
    }
}

async fn load_job(pool: &PgPool, id: Uuid) -> Job {
    sqlx::query_as("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the job row must be readable")
}

/// Makes a `retry_wait` job due without waiting for the wall clock.
async fn make_due(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE jobs SET available_at = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("the availability must be adjustable");
}

fn finish(
    job_id: Uuid,
    lease_token: Uuid,
    outcome: ExecutionOutcome,
    cancel_confirmed: bool,
) -> FinishExecution {
    FinishExecution {
        job_id,
        lease_token,
        outcome,
        cancel_confirmed,
    }
}

#[tokio::test]
async fn t08_1_total_attempts_capped_at_four() {
    let db = test_db().await;
    let (ctx, _clock) = test_context(&db.pool).await;
    let job = enqueue(&db.pool, 4).await;

    let mut attempts = Vec::new();
    for expected_attempt in 1..=4 {
        let lease = claim(&db.pool).await;
        assert_eq!(lease.job_id, job.id);
        assert_eq!(
            lease.attempt, expected_attempt,
            "each claim consumes one attempt"
        );
        attempts.push(lease.attempt);

        let outcome = ExecutionOutcome::TemporaryFailure {
            message: format!("transient failure {expected_attempt}"),
        };

        if expected_attempt < 4 {
            let retried =
                service::finish_execution(&ctx, finish(job.id, lease.token, outcome, false))
                    .await
                    .expect("a temporary failure below the budget must schedule a retry");

            assert_eq!(retried.state, "retry_wait");
            assert_eq!(
                retried.attempts, expected_attempt,
                "scheduling a retry must not spend another attempt"
            );
            assert_eq!(
                retried.available_at - ctx.now(),
                Duration::seconds(BACKOFF_SECONDS[expected_attempt as usize - 1]),
                "attempt {expected_attempt} must back off by the contract delay"
            );
            assert_eq!(
                retried.last_error.as_deref(),
                Some(format!("transient failure {expected_attempt}").as_str())
            );
            assert!(
                jobs::claim(&db.pool, "worker-a")
                    .await
                    .expect("claim must not fail")
                    .is_none(),
                "a retry_wait job must not be claimable before its delay elapsed"
            );

            make_due(&db.pool, job.id).await;
        } else {
            let failed =
                service::finish_execution(&ctx, finish(job.id, lease.token, outcome, false))
                    .await
                    .expect("the last temporary failure must become terminal");

            assert_eq!(
                failed.state, "failed",
                "the fourth attempt is the last one in the budget"
            );
            assert_eq!(failed.attempts, 4);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("transient failure 4"),
                "the terminal failure keeps the last error"
            );
            assert!(
                jobs::claim(&db.pool, "worker-a")
                    .await
                    .expect("claim must not fail")
                    .is_none(),
                "a failed job must not be claimed a fifth time"
            );
        }
    }

    assert_eq!(attempts, vec![1, 2, 3, 4]);

    let stored = load_job(&db.pool, job.id).await;
    assert_eq!(stored.state, "failed");
    assert_eq!(stored.attempts, 4);
    assert_eq!(stored.lease_token, None, "a terminal job carries no lease");

    db.drop().await;
}

#[tokio::test]
async fn t08_2_permanent_failure_is_terminal() {
    let db = test_db().await;
    let (ctx, _clock) = test_context(&db.pool).await;
    let job = enqueue(&db.pool, 4).await;
    let lease = claim(&db.pool).await;

    let failed = service::finish_execution(
        &ctx,
        finish(
            job.id,
            lease.token,
            ExecutionOutcome::PermanentFailure {
                message: "unsupported document format".to_owned(),
            },
            false,
        ),
    )
    .await
    .expect("a permanent failure must be recorded");

    assert_eq!(failed.state, "failed", "a permanent error never retries");
    assert_eq!(failed.attempts, 1, "the first attempt was the only one");
    assert_eq!(
        failed.last_error.as_deref(),
        Some("unsupported document format")
    );
    assert_eq!(failed.lease_token, None);
    assert!(
        jobs::claim(&db.pool, "worker-a")
            .await
            .expect("claim must not fail")
            .is_none(),
        "a permanent failure must not leave a retry_wait job behind"
    );

    db.drop().await;
}

#[tokio::test]
async fn t08_3_unconfirmed_cancel_keeps_slot() {
    let db = test_db().await;
    let (ctx, _clock) = test_context(&db.pool).await;

    // A running job is asked to cancel: the lease survives until the worker
    // confirms that the execution really ended.
    let job = enqueue(&db.pool, 4).await;
    let lease = claim(&db.pool).await;
    assert_eq!(lease.state, knoted_core::jobs::JobState::Running);

    let requested = service::request_cancel(&ctx, job.id)
        .await
        .expect("cancelling a running job must work");
    assert_eq!(requested.state, "cancel_requested");
    assert_eq!(
        requested.lease_token,
        Some(lease.token),
        "a cancellation request must keep the lease"
    );

    let unconfirmed = service::finish_execution(
        &ctx,
        finish(job.id, lease.token, ExecutionOutcome::Succeeded, false),
    )
    .await
    .expect("reporting an unconfirmed cancellation must not fail");
    assert_eq!(
        unconfirmed.state, "cancel_requested",
        "an unconfirmed cancellation must not write a success"
    );
    assert_eq!(
        unconfirmed.lease_token,
        Some(lease.token),
        "the slot stays allocated until the child process exit is confirmed"
    );

    // The live lease keeps the job out of the queue, so no other worker can
    // start a second execution while the first one may still be running.
    assert!(
        jobs::claim(&db.pool, "worker-b")
            .await
            .expect("claim must not fail")
            .is_none(),
        "an unconfirmed cancellation must not release the slot"
    );
    // Repeating the request is idempotent, not an error.
    let repeated = service::request_cancel(&ctx, job.id)
        .await
        .expect("a repeated cancel request must be idempotent");
    assert_eq!(repeated.state, "cancel_requested");

    let cancelled = service::finish_execution(
        &ctx,
        finish(job.id, lease.token, ExecutionOutcome::Succeeded, true),
    )
    .await
    .expect("a confirmed cancellation must be recorded");
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(
        cancelled.lease_token, None,
        "confirmation releases the lease"
    );
    assert_eq!(cancelled.locked_by, None);

    // The old token must never write anything afterwards.
    let replay = service::finish_execution(
        &ctx,
        finish(job.id, lease.token, ExecutionOutcome::Succeeded, true),
    )
    .await;
    assert!(
        matches!(replay, Err(DomainJobError::LeaseLost { job_id }) if job_id == job.id),
        "a stale token must be rejected, got {replay:?}"
    );
    assert_eq!(load_job(&db.pool, job.id).await.state, "cancelled");

    // A queued job has no lease, so it must go straight to `cancelled` instead
    // of the impossible `cancel_requested` state.
    let queued = enqueue(&db.pool, 4).await;
    let cancelled = service::request_cancel(&ctx, queued.id)
        .await
        .expect("a queued job must be cancellable");
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.lease_token, None);
    assert!(
        jobs::claim(&db.pool, "worker-b")
            .await
            .expect("claim must not fail")
            .is_none(),
        "a cancelled queued job must not run"
    );

    // A retry_wait job must be cancellable too, and is due again only if it was
    // not cancelled.
    let waiting = enqueue(&db.pool, 4).await;
    let waiting_lease = claim(&db.pool).await;
    let waiting = service::finish_execution(
        &ctx,
        finish(
            waiting.id,
            waiting_lease.token,
            ExecutionOutcome::TemporaryFailure {
                message: "transient".to_owned(),
            },
            false,
        ),
    )
    .await
    .expect("a temporary failure must schedule a retry");
    assert_eq!(waiting.state, "retry_wait");
    let cancelled = service::request_cancel(&ctx, waiting.id)
        .await
        .expect("a waiting retry must be cancellable");
    assert_eq!(cancelled.state, "cancelled");
    make_due(&db.pool, waiting.id).await;
    assert!(
        jobs::claim(&db.pool, "worker-b")
            .await
            .expect("claim must not fail")
            .is_none(),
        "a cancelled retry must never become due"
    );

    // A takeover of a cancellation request must tell the new worker that it is
    // supposed to confirm a cancellation, not execute the work.
    let takeover_job = enqueue(&db.pool, 4).await;
    let stale = claim(&db.pool).await;
    service::request_cancel(&ctx, takeover_job.id)
        .await
        .expect("the running job must be cancellable");
    sqlx::query("UPDATE jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(takeover_job.id)
        .execute(&db.pool)
        .await
        .expect("the lease must be expirable");
    let takeover = jobs::claim(&db.pool, "worker-b")
        .await
        .expect("the takeover claim must not fail")
        .expect("an expired cancellation lease must be takeable over");
    assert_eq!(
        takeover.state,
        knoted_core::jobs::JobState::CancelRequested,
        "the takeover lease must carry the cancellation intent"
    );
    assert_ne!(takeover.token, stale.token);

    db.drop().await;
}

#[tokio::test]
async fn t08_4_manual_retry_links_original() {
    let db = test_db().await;
    let (ctx, _clock) = test_context(&db.pool).await;

    let original = enqueue(&db.pool, 4).await;
    let lease = claim(&db.pool).await;
    let original = service::finish_execution(
        &ctx,
        finish(
            original.id,
            lease.token,
            ExecutionOutcome::PermanentFailure {
                message: "bad input".to_owned(),
            },
            false,
        ),
    )
    .await
    .expect("the original job must fail");

    let retried = service::retry_failed(&ctx, original.id)
        .await
        .expect("a failed job must be manually retryable");
    assert_ne!(retried.id, original.id, "a manual retry is a new job");
    assert_eq!(retried.retry_of, Some(original.id));
    assert_eq!(retried.kind, original.kind);
    assert_eq!(retried.payload, original.payload);
    assert_eq!(retried.state, "queued");
    assert_eq!(retried.attempts, 0, "the new job starts a fresh budget");
    assert_eq!(
        retried.idempotency_key, None,
        "a retry must not reuse the original deduplication key"
    );

    let unchanged = load_job(&db.pool, original.id).await;
    assert_eq!(
        unchanged.state, "failed",
        "manual retry must not resurrect the original row"
    );
    assert_eq!(unchanged.attempts, 1);
    assert_eq!(unchanged.retry_of, None);

    // A cancelled job is retryable too.
    let cancelled = enqueue(&db.pool, 4).await;
    service::request_cancel(&ctx, cancelled.id)
        .await
        .expect("a queued job must be cancellable");
    let retried_cancelled = service::retry_failed(&ctx, cancelled.id)
        .await
        .expect("a cancelled job must be manually retryable");
    assert_eq!(retried_cancelled.retry_of, Some(cancelled.id));

    // A live job must not be duplicated by a manual retry: that would run the
    // same work twice.
    let running = enqueue(&db.pool, 4).await;
    let running_lease = claim_job(&db.pool, running.id).await;
    let rejected = service::retry_failed(&ctx, running.id).await;
    assert!(
        matches!(
            rejected,
            Err(DomainJobError::NotRetryable { job_id, .. }) if job_id == running.id
        ),
        "a running job must not be retried, got {rejected:?}"
    );
    assert_eq!(load_job(&db.pool, running.id).await.state, "running");
    assert_eq!(
        load_job(&db.pool, running.id).await.lease_token,
        Some(running_lease.token),
        "a rejected retry must not touch the live lease"
    );

    // A missing id is a not-found error, not a silent success.
    let missing = service::retry_failed(&ctx, Uuid::new_v4()).await;
    assert!(
        matches!(missing, Err(DomainJobError::NotFound(id)) if id != running.id),
        "a missing job must report not found, got {missing:?}"
    );

    db.drop().await;
}
