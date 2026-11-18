use knoted_core::{
    config::FilesConfig, context::AppContext, db::testing::TestDb, files::scanner::enqueue_scan,
};
use knoted_worker::{
    handlers::scan::Dispatcher,
    runner::{StepOutcome, WorkerRunner},
};
#[tokio::test]
async fn t46_dispatcher_captures_after_scan_via_shared_queue() {
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
    let runner = WorkerRunner::with_context(ctx.clone(), "t46", Dispatcher { ctx: ctx.clone() });
    let scan = Box::pin(runner.step()).await.unwrap();
    let capture = Box::pin(runner.step()).await.unwrap();
    let extraction = Box::pin(runner.step()).await.unwrap();
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE kind='file.extract' AND state='succeeded'",
    )
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
    assert!(
        matches!(extraction,StepOutcome::Finished{job,..} if job.state=="succeeded"),
        "real extraction must run"
    );
    assert_eq!(rows, 1);
}

fn temp() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&p).unwrap();
    p
}

async fn setup(
    bytes: &[u8],
    name: &str,
) -> (TestDb, AppContext, std::path::PathBuf, std::path::PathBuf) {
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    std::fs::write(root.join(name), bytes).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.clone()),
        staging_root: Some(staging.clone()),
        ..Default::default()
    });
    enqueue_scan(&ctx).await.unwrap();
    (db, ctx, root, staging)
}
#[tokio::test]
async fn t46_invalid_encoding_and_nul_are_permanent_empty_is_success() {
    for (bytes, expected) in [
        (&b""[..], "succeeded"),
        (&b"\xff"[..], "failed"),
        (&b"a\0b"[..], "failed"),
    ] {
        let (db, ctx, root, staging) = setup(bytes, "sample.txt").await;
        let runner = WorkerRunner::with_context(
            ctx.clone(),
            "t46-terminal",
            Dispatcher { ctx: ctx.clone() },
        );
        Box::pin(runner.step()).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        let outcome = Box::pin(runner.step()).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
        db.drop().await;
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(staging).unwrap();
        assert!(
            matches!(outcome,StepOutcome::Finished{job,..} if job.state==expected && job.attempts==1)
        );
        assert_eq!(rows, if expected == "succeeded" { 1 } else { 0 });
    }
}
#[tokio::test]
async fn t46_existing_capture_backfills_and_metadata_replay_keeps_success() {
    let (db, ctx, root, staging) = setup(b"existing", "README").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-existing", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    // Model a pre-T46 closed source: remove only its never-started extraction job.
    sqlx::query("DELETE FROM jobs WHERE kind='file.extract'")
        .execute(ctx.pool())
        .await
        .unwrap();
    enqueue_scan(&ctx).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let result = Box::pin(runner.step()).await.unwrap();
    let before: (uuid::Uuid, i64) =
        sqlx::query_as("SELECT id,content_version FROM file_extractions")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    sqlx::query("UPDATE file_sources SET version=version+1,note='metadata only'")
        .execute(ctx.pool())
        .await
        .unwrap();
    enqueue_scan(&ctx).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let after: Vec<(uuid::Uuid, i64)> =
        sqlx::query_as("SELECT id,content_version FROM file_extractions")
            .fetch_all(ctx.pool())
            .await
            .unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(result,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert_eq!(after, vec![before]);
    assert_eq!(jobs, 1);
}
#[tokio::test]
async fn t46_superseded_queued_attempt_does_not_block_new_observation() {
    let (db, ctx, root, staging) = setup(b"old", "sample.txt").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-stale", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    std::fs::write(root.join("sample.txt"), b"new body").unwrap();
    enqueue_scan(&ctx).await.unwrap();
    sqlx::query("UPDATE jobs SET priority=10 WHERE kind='file.scan' AND state='queued'")
        .execute(ctx.pool())
        .await
        .unwrap();
    Box::pin(runner.step()).await.unwrap();
    let old = Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let new = Box::pin(runner.step()).await.unwrap();
    let bodies: Vec<String> = sqlx::query_scalar("SELECT blocks->0->>'text' FROM file_extractions")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(old,StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert!(matches!(new,StepOutcome::Finished{job,..} if job.state=="succeeded"));
    assert_eq!(bodies, vec!["new body"]);
}
#[tokio::test]
async fn t46_later_binary_formats_never_become_text_success() {
    let (db, ctx, root, staging) = setup(b"plain bytes named pdf", "sample.PDF").await;
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-format", Dispatcher { ctx: ctx.clone() });
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract' AND payload->>'parser_revision'='rust-text-v1'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let parser: String = sqlx::query_scalar(
        "SELECT payload->>'parser_revision' FROM jobs WHERE kind='file.extract'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    assert_eq!(parser, knoted_core::files::pdf::PARSER_REVISION);
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn t46_running_text_cancellation_publishes_nothing() {
    let bytes = vec![b'x'; 100 * 1024 * 1024];
    let (db, ctx, root, staging) = setup(&bytes, "large.txt").await;
    drop(bytes);
    let runner =
        WorkerRunner::with_context(ctx.clone(), "t46-cancel", Dispatcher { ctx: ctx.clone() })
            .with_heartbeat_interval(std::time::Duration::from_millis(1));
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let cancel = async {
        let id = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let id: Option<uuid::Uuid> = sqlx::query_scalar(
                    "SELECT id FROM jobs WHERE kind='file.extract' AND state='running'",
                )
                .fetch_optional(ctx.pool())
                .await
                .unwrap();
                if let Some(id) = id {
                    break id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        knoted_core::jobs::service::request_cancel(&ctx, id)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(Box::pin(runner.step()), cancel);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_extractions")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(matches!(result.unwrap(),StepOutcome::Finished{job,..} if job.state=="cancelled"));
    assert_eq!(count, 0);
}

async fn unchanged_failed_input_is_not_automatically_requeued(
    bytes: &[u8],
    exhaust_io: bool,
    legacy: bool,
) {
    let (db, ctx, root, staging) = setup(bytes, "failed.txt").await;
    let runner = WorkerRunner::with_context(
        ctx.clone(),
        "t46-terminal-rescan",
        Dispatcher { ctx: ctx.clone() },
    );
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    if exhaust_io {
        let object: String =
            sqlx::query_scalar("SELECT input_object_id FROM file_sources WHERE status='present'")
                .fetch_one(ctx.pool())
                .await
                .unwrap();
        std::fs::remove_file(staging.join("input").join(object)).unwrap();
        sqlx::query("UPDATE jobs SET max_attempts=1 WHERE kind='file.extract'")
            .execute(ctx.pool())
            .await
            .unwrap();
    }
    let first = Box::pin(runner.step()).await.unwrap();
    if legacy {
        sqlx::query("UPDATE jobs SET payload=payload-'parser_revision'-'options_hash' WHERE kind='file.extract'").execute(ctx.pool()).await.unwrap();
    }
    type Snapshot = (uuid::Uuid, i64, i64, i64, uuid::Uuid, String, bool, i64);
    let query = "SELECT id,version,content_version,observation_version,captured_scan_id,sha256,opaque_handle IS NOT NULL,(SELECT count(*) FROM jobs WHERE kind='file.extract') FROM file_sources WHERE status='present'";
    let before: Snapshot = sqlx::query_as(query).fetch_one(ctx.pool()).await.unwrap();
    let jobs_before: Vec<(uuid::Uuid, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT id,state,attempts,idempotency_key FROM jobs WHERE kind='file.extract' ORDER BY created_at,id",
    ).fetch_all(ctx.pool()).await.unwrap();
    enqueue_scan(&ctx).await.unwrap();
    let scan = Box::pin(runner.step()).await.unwrap();
    let capture = Box::pin(runner.step()).await.unwrap();
    let after: Snapshot = sqlx::query_as(query).fetch_one(ctx.pool()).await.unwrap();
    let jobs_after: Vec<(uuid::Uuid, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT id,state,attempts,idempotency_key FROM jobs WHERE kind='file.extract' ORDER BY created_at,id",
    ).fetch_all(ctx.pool()).await.unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    println!(
        "snapshot fields: source_id/version/content_version/observation_version/captured_scan_id/SHA/strong_handle/extract_jobs"
    );
    println!(
        "before={before:?}\nafter={after:?}\njobs_before={jobs_before:?}\njobs_after={jobs_after:?}"
    );
    assert!(
        matches!(first, StepOutcome::Finished { job, .. } if job.state == "failed" && job.attempts == 1)
    );
    assert!(matches!(scan, StepOutcome::Finished { job, .. } if job.state == "succeeded"));
    assert!(matches!(capture, StepOutcome::Finished { job, .. } if job.state == "succeeded"));
    assert!(
        before.6 && after.6,
        "fixture must have a real strong identity"
    );
    assert_eq!(before.0, after.0, "same source identity");
    assert_eq!(
        (before.1, before.2, before.3),
        (after.1, after.2, after.3),
        "unchanged metadata/content/observation versions"
    );
    assert_eq!(before.5, after.5, "unchanged bytes and SHA");
    assert_ne!(
        before.4, after.4,
        "must exercise a new complete scan capture"
    );
    assert_eq!(before.7, 1);
    assert_eq!(
        after.7, 1,
        "ordinary unchanged rescan must not reset terminal extraction failure or retry budget"
    );
    assert_eq!(jobs_before, jobs_after);
}

#[tokio::test]
async fn t46_failed_invalid_encoding_waits_for_manual_retry_after_unchanged_rescan() {
    unchanged_failed_input_is_not_automatically_requeued(b"\xff", false, false).await;
}

#[tokio::test]
async fn t46_failed_nul_waits_for_manual_retry_after_unchanged_rescan() {
    unchanged_failed_input_is_not_automatically_requeued(b"a\0b", false, false).await;
}

#[tokio::test]
async fn t46_failed_temporary_io_budget_exhausted_is_not_reopened_by_rescan() {
    unchanged_failed_input_is_not_automatically_requeued(b"valid text", true, false).await;
}

#[tokio::test]
async fn t46_failed_previous_content_does_not_block_changed_content() {
    let (db, ctx, root, staging) = setup(b"\xff", "changing.txt").await;
    let runner = WorkerRunner::with_context(
        ctx.clone(),
        "t46-new-content",
        Dispatcher { ctx: ctx.clone() },
    );
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let before:(uuid::Uuid,i64,bool)=sqlx::query_as("SELECT id,content_version,opaque_handle IS NOT NULL FROM file_sources WHERE status='present'").fetch_one(ctx.pool()).await.unwrap();
    std::fs::write(root.join("changing.txt"), b"valid new body").unwrap();
    enqueue_scan(&ctx).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    Box::pin(runner.step()).await.unwrap();
    let result = Box::pin(runner.step()).await.unwrap();
    let after: (uuid::Uuid, i64) =
        sqlx::query_as("SELECT id,content_version FROM file_sources WHERE status='present'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    let bodies: Vec<String> = sqlx::query_scalar("SELECT blocks->0->>'text' FROM file_extractions")
        .fetch_all(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(staging).unwrap();
    assert!(before.2);
    assert_eq!(before.0, after.0);
    assert_eq!(after.1, before.1 + 1);
    assert_eq!(bodies, vec!["valid new body"]);
    assert!(matches!(result,StepOutcome::Finished{job,..}if job.state=="succeeded"));
}

#[tokio::test]
async fn t46_failed_other_parser_or_options_do_not_block_current_semantics() {
    for key in ["parser_revision", "options_hash"] {
        let (db, ctx, root, staging) = setup(b"\xff", "parser.txt").await;
        let runner = WorkerRunner::with_context(
            ctx.clone(),
            "t46-new-parser",
            Dispatcher { ctx: ctx.clone() },
        );
        Box::pin(runner.step()).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        let original:(Option<String>,Option<String>)=sqlx::query_as("SELECT payload->>'parser_revision',payload->>'options_hash' FROM jobs WHERE kind='file.extract'").fetch_one(ctx.pool()).await.unwrap();
        sqlx::query("UPDATE jobs SET payload=jsonb_set(payload,ARRAY[$1::text],to_jsonb($2::text)) WHERE kind='file.extract'").bind(key).bind(if key=="options_hash"{"0000000000000000000000000000000000000000000000000000000000000000"}else{"previous-parser"}).execute(ctx.pool()).await.unwrap();
        enqueue_scan(&ctx).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        Box::pin(runner.step()).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.extract'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
        db.drop().await;
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(staging).unwrap();
        assert_eq!(
            original.0.as_deref(),
            Some("rust-text-v1"),
            "queued job must declare parser identity"
        );
        assert_eq!(
            original.1.as_deref(),
            Some("6541278e7ee4c2460f494c6f29778d64d8fb1f81d5fe19a1ae7e4d914b60272f"),
            "queued job must declare options identity"
        );
        assert_eq!(
            count, 2,
            "a failure of {key} must not suppress a different semantic extraction"
        );
    }
}

#[tokio::test]
async fn t46_failed_legacy_v1_job_without_explicit_parser_metadata_stays_terminal() {
    unchanged_failed_input_is_not_automatically_requeued(b"\xff", false, true).await;
}
