//! T07 acceptance tests: job queue primitives, lease claiming and the
//! token-guarded completion primitive.
//!
//! The tests run against the real local PostgreSQL test database
//! (`KNOTED_TEST_DATABASE_URL`, default
//! `postgres:///knoted_test?host=/run/postgresql`) with a per-test schema, so
//! the default parallel harness is safe. A missing database is a failure, never
//! a skip.
//!
//! Covered here: a job is claimed by exactly one worker, an expired lease can be
//! taken over, and a stale token can neither heartbeat nor finish the job. The
//! retry/backoff state machine is T08 and deliberately absent.

use chrono::{DateTime, Utc};
use knoted_core::db::testing::TestDb;
use knoted_core::jobs::{self, Job, JobOutcome, JobSpec};
use sqlx::PgPool;
use uuid::Uuid;

const UNREACHABLE_HINT: &str = "the local PostgreSQL test database must be running \
     (KNOTED_TEST_DATABASE_URL, default postgres:///knoted_test?host=/run/postgresql)";

async fn test_db() -> TestDb {
    TestDb::create()
        .await
        .unwrap_or_else(|error| panic!("schema setup failed: {error}; {UNREACHABLE_HINT}"))
}

/// Enqueues one job in its own transaction, as a service would.
async fn enqueue(pool: &PgPool, spec: JobSpec) -> Job {
    let mut tx = pool.begin().await.expect("a transaction must start");
    let job = jobs::enqueue(&mut tx, spec).await.expect("enqueue");
    tx.commit()
        .await
        .expect("the enqueue transaction must commit");
    job
}

fn parse_spec(payload: &str) -> JobSpec {
    JobSpec {
        kind: "parse".to_owned(),
        idempotency_key: None,
        payload: serde_json::json!({ "input_object_id": payload }),
        priority: 0,
        max_attempts: 4,
        available_at: None,
    }
}

/// Reads the persisted row so assertions compare database state, not just the
/// values returned by the API.
async fn load_job(pool: &PgPool, id: Uuid) -> (String, i32, Option<Uuid>, Option<String>) {
    sqlx::query_as("SELECT state, attempts, lease_token, locked_by FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the job row must be readable")
}

async fn outbox_count(pool: &PgPool, job_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox WHERE job_id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .expect("outbox must be readable")
}

#[tokio::test]
async fn t07_1_two_workers_do_not_claim_same_job() {
    let db = test_db().await;
    let job = enqueue(&db.pool, parse_spec("object-1")).await;
    assert_eq!(job.state, "queued");
    assert_eq!(job.attempts, 0);
    assert_eq!(
        outbox_count(&db.pool, job.id).await,
        1,
        "enqueueing writes exactly one outbox row"
    );

    // Two workers race for the single queued job. `FOR UPDATE SKIP LOCKED` must
    // give it to one of them and leave the other with nothing to do.
    let (first, second) = tokio::join!(
        jobs::claim(&db.pool, "worker-a"),
        jobs::claim(&db.pool, "worker-b")
    );
    let first = first.expect("the first claim must not fail");
    let second = second.expect("the second claim must not fail");

    let lease = match (first, second) {
        (Some(lease), None) | (None, Some(lease)) => lease,
        (Some(_), Some(_)) => panic!("two workers must never claim the same job"),
        (None, None) => panic!("a queued job must be claimed by one of the two workers"),
    };
    assert_eq!(lease.job_id, job.id);
    assert_eq!(lease.attempt, 1, "the claim counts as the first attempt");
    assert!(
        lease.worker == "worker-a" || lease.worker == "worker-b",
        "the lease must belong to one of the racing workers, got `{}`",
        lease.worker
    );

    let (state, attempts, token, locked_by) = load_job(&db.pool, job.id).await;
    assert_eq!(state, "running");
    assert_eq!(attempts, 1);
    assert_eq!(token, Some(lease.token), "the lease token is persisted");
    assert_eq!(locked_by.as_deref(), Some(lease.worker.as_str()));

    // A running job with a live lease is not claimable again.
    let again = jobs::claim(&db.pool, "worker-c")
        .await
        .expect("a third claim must not fail");
    assert!(again.is_none(), "a live lease must not be stolen");

    db.drop().await;
}

#[tokio::test]
async fn t07_2_expired_lease_is_taken_over() {
    let db = test_db().await;
    let job = enqueue(&db.pool, parse_spec("object-2")).await;
    let stale = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("the first claim must not fail")
        .expect("a queued job must be claimable");

    // Simulate a worker that stopped heartbeating: its lease expired.
    let expired_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE jobs SET lease_expires_at = now() - interval '1 second' \
         WHERE id = $1 RETURNING lease_expires_at",
    )
    .bind(job.id)
    .fetch_one(&db.pool)
    .await
    .expect("the lease must be expirable");

    let takeover = jobs::claim(&db.pool, "worker-b")
        .await
        .expect("the takeover claim must not fail")
        .expect("an expired lease must be claimable by another worker");

    assert_eq!(takeover.job_id, job.id);
    assert_eq!(takeover.worker, "worker-b");
    assert_eq!(takeover.attempt, 2, "a takeover counts as another attempt");
    assert_ne!(
        takeover.token, stale.token,
        "a takeover must issue a fresh lease token"
    );
    assert!(
        takeover.expires_at > expired_at,
        "the takeover must install a new expiry in the future"
    );

    let (state, attempts, token, locked_by) = load_job(&db.pool, job.id).await;
    assert_eq!(state, "running");
    assert_eq!(attempts, 2);
    assert_eq!(token, Some(takeover.token));
    assert_eq!(locked_by.as_deref(), Some("worker-b"));

    db.drop().await;
}

#[tokio::test]
async fn t07_3_stale_token_cannot_finish() {
    let db = test_db().await;
    let job = enqueue(&db.pool, parse_spec("object-3")).await;
    let stale = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("the first claim must not fail")
        .expect("a queued job must be claimable");

    sqlx::query("UPDATE jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await
        .expect("the lease must be expirable");
    let takeover = jobs::claim(&db.pool, "worker-b")
        .await
        .expect("the takeover claim must not fail")
        .expect("an expired lease must be takeable over");

    // The previous worker wakes up and tries to continue with its old token.
    assert!(
        !jobs::heartbeat(&db.pool, &stale)
            .await
            .expect("heartbeat must not fail"),
        "a stale token must not extend the new lease"
    );
    assert!(
        !jobs::finish_with_token(&db.pool, &stale, JobOutcome::Succeeded)
            .await
            .expect("finish must not fail"),
        "a stale token must not complete the job"
    );

    let (state, attempts, token, locked_by) = load_job(&db.pool, job.id).await;
    assert_eq!(
        state, "running",
        "the stale finish must not change the state"
    );
    assert_eq!(attempts, 2);
    assert_eq!(token, Some(takeover.token), "the new lease stays intact");
    assert_eq!(locked_by.as_deref(), Some("worker-b"));

    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&db.pool)
            .await
            .expect("the job row must be readable");
    assert_eq!(last_error, None, "the stale finish must not write an error");

    // The current holder can heartbeat and complete.
    assert!(
        jobs::heartbeat(&db.pool, &takeover)
            .await
            .expect("heartbeat must not fail"),
        "the current token must own the lease"
    );
    assert!(
        jobs::finish_with_token(
            &db.pool,
            &takeover,
            JobOutcome::Failed {
                error: "parser rejected the input".to_owned(),
            }
        )
        .await
        .expect("finish must not fail")
    );
    let (state, attempts, token, locked_by) = load_job(&db.pool, job.id).await;
    assert_eq!(state, "failed");
    assert_eq!(attempts, 2);
    assert_eq!(token, None, "finishing releases the lease token");
    assert_eq!(locked_by, None);
    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&db.pool)
            .await
            .expect("the job row must be readable");
    assert_eq!(last_error.as_deref(), Some("parser rejected the input"));

    db.drop().await;
}

#[tokio::test]
async fn t07_4_enqueue_is_idempotent() {
    let db = test_db().await;
    let spec = || JobSpec {
        idempotency_key: Some("file-42-v3".to_owned()),
        ..parse_spec("object-4")
    };

    let first = enqueue(&db.pool, spec()).await;
    let topic: String = sqlx::query_scalar("SELECT topic FROM outbox WHERE job_id = $1")
        .bind(first.id)
        .fetch_one(&db.pool)
        .await
        .expect("the outbox event must be readable");
    assert_eq!(
        topic, "job.enqueued.parse",
        "the outbox topic is derived from the job kind"
    );

    let second = enqueue(&db.pool, spec()).await;
    assert_eq!(
        second.id, first.id,
        "the same (kind, idempotency_key) must return the existing job"
    );
    assert_eq!(
        outbox_count(&db.pool, first.id).await,
        1,
        "a duplicate enqueue must not publish a second event"
    );

    // A different kind with the same key is a different job.
    let other_kind = JobSpec {
        kind: "embed".to_owned(),
        ..spec()
    };
    let other = enqueue(&db.pool, other_kind).await;
    assert_ne!(other.id, first.id);

    // Jobs without an idempotency key are never deduplicated.
    let anonymous_a = enqueue(&db.pool, parse_spec("object-5")).await;
    let anonymous_b = enqueue(&db.pool, parse_spec("object-5")).await;
    assert_ne!(anonymous_a.id, anonymous_b.id);

    db.drop().await;
}

#[tokio::test]
async fn t07_4_rejects_unknown_state() {
    let db = test_db().await;
    let job = enqueue(&db.pool, parse_spec("object-6")).await;

    let rejected = sqlx::query("UPDATE jobs SET state = 'paused' WHERE id = $1")
        .bind(job.id)
        .execute(&db.pool)
        .await;
    assert!(
        rejected.is_err(),
        "the state check constraint must reject unknown states"
    );

    db.drop().await;
}

#[tokio::test]
async fn t07_1_claim_honours_priority_and_availability() {
    let db = test_db().await;
    let later = JobSpec {
        available_at: Some(Utc::now() + chrono::Duration::seconds(60)),
        ..parse_spec("object-7")
    };
    let low = JobSpec {
        priority: 1,
        ..parse_spec("object-8")
    };
    let high = JobSpec {
        priority: 9,
        ..parse_spec("object-9")
    };
    let not_yet = enqueue(&db.pool, later).await;
    let low = enqueue(&db.pool, low).await;
    let high = enqueue(&db.pool, high).await;

    let first = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("claim must not fail")
        .expect("a queued job must be claimable");
    assert_eq!(first.job_id, high.id, "priority order must be respected");
    let second = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("claim must not fail")
        .expect("a second queued job must be claimable");
    assert_eq!(second.job_id, low.id);

    let third = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("claim must not fail");
    assert!(
        third.is_none(),
        "a job that is not yet available must not be claimed ({not_yet:?})"
    );

    // The delayed job becomes claimable once its `available_at` passes.
    sqlx::query("UPDATE jobs SET available_at = now() - interval '1 second' WHERE id = $1")
        .bind(not_yet.id)
        .execute(&db.pool)
        .await
        .expect("the availability must be adjustable");
    let third = jobs::claim(&db.pool, "worker-a")
        .await
        .expect("claim must not fail")
        .expect("an available job must be claimable");
    assert_eq!(third.job_id, not_yet.id);
    assert_eq!(third.attempt, 1);

    db.drop().await;
}

#[tokio::test]
async fn t07_2_claim_reports_empty_queue() {
    let db = test_db().await;
    assert!(
        jobs::claim(&db.pool, "worker-a")
            .await
            .expect("claim must not fail")
            .is_none(),
        "an empty queue must return None instead of blocking"
    );
    db.drop().await;
}
