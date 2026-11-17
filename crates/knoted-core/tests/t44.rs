use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    db::testing::TestDb,
    files::{
        capture::{SourceObservation, capture_stable},
        safe_fs,
        scanner::{ObserveRoot, observe_root, reconcile_complete_scan},
    },
};
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;
fn temp() -> TempDir {
    tempfile::tempdir_in("/data/knoted-dev/tasks/t44/tmp").unwrap()
}
async fn lease(ctx: &AppContext) -> knoted_core::jobs::Lease {
    let mut tx = ctx.pool().begin().await.unwrap();
    knoted_core::jobs::enqueue(&mut tx, knoted_core::jobs::JobSpec::new("test.capture"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    knoted_core::jobs::claim(ctx.pool(), "t44")
        .await
        .unwrap()
        .unwrap()
}
async fn fixture(size: usize) -> (TestDb, AppContext, TempDir, TempDir, SourceObservation) {
    let db = TestDb::create().await.unwrap();
    let root = temp();
    let staging = temp();
    std::fs::write(root.path().join("a.txt"), vec![b'a'; size]).unwrap();
    let ctx = AppContext::new(db.pool.clone()).with_files(FilesConfig {
        enabled: true,
        root: Some(root.path().into()),
        staging_root: Some(staging.path().into()),
        ..Default::default()
    });
    let l = lease(&ctx).await;
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: l.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let scan_id = report.scan_id;
    let root_id = report.root_id;
    let generation = report.generation;
    assert!(reconcile_complete_scan(&ctx, report).await.unwrap());
    let fd = safe_fs::open_root(root.path()).unwrap();
    let file = std::fs::File::open(root.path().join("a.txt")).unwrap();
    let input = SourceObservation {
        scan_id,
        root_id,
        generation,
        root_path: root.path().into(),
        root_identity: safe_fs::metadata(fd).unwrap().identity,
        relative_path: PathBuf::from("a.txt"),
        metadata: safe_fs::metadata(file).unwrap(),
        lease: l,
        cancel: Arc::default(),
    };
    (db, ctx, root, staging, input)
}
#[tokio::test]
async fn t44_stable_registers_closed_input_and_registry() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    let started = std::time::Instant::now();
    let result = capture_stable(&ctx, input).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    let object = result.expect("stable input must be captured");
    assert!(started.elapsed() >= std::time::Duration::from_secs(2));
    assert_eq!(object.size, 3);
    assert_eq!(
        std::fs::read(staging.path().join("input").join(object.object_id.as_str())).unwrap(),
        b"aaa"
    );
    assert_eq!(rows, 1);
}
#[tokio::test]
async fn t44_scan_atomically_enqueues_capture_jobs() {
    let (db, ctx, _root, _staging, _input) = fixture(3).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind='file.capture'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert_eq!(count, 1, "complete scan must durably enqueue capture");
}

fn files(staging: &TempDir) -> Vec<PathBuf> {
    std::fs::read_dir(staging.path().join("input"))
        .into_iter()
        .flatten()
        .map(|e| e.unwrap().path())
        .collect()
}
async fn wait_part(staging: &TempDir) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if files(staging)
                .iter()
                .any(|p| p.extension().is_some_and(|e| e == "part"))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("copy must start");
}
#[tokio::test]
async fn t44_1_writes_between_stats_reject_stale_capture() {
    let (db, ctx, root, staging, input) = fixture(4).await;
    let change = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::fs::write(root.path().join("a.txt"), "new").unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), change);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(result.is_err());
    assert_eq!(rows, 0);
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_2_fifo_replacement_is_nonblocking() {
    let (db, ctx, root, staging, input) = fixture(4).await;
    std::fs::remove_file(root.path().join("a.txt")).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        root.path().join("a.txt"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        capture_stable(&ctx, input),
    )
    .await;
    db.drop().await;
    assert!(result.unwrap().is_err());
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_3_admission_over_100mib_leaves_no_copy() {
    let (db, ctx, root, staging, mut input) = fixture(4).await;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(root.path().join("a.txt"))
        .unwrap();
    f.set_len(100 * 1024 * 1024 + 1).unwrap();
    input.metadata = safe_fs::metadata(f).unwrap();
    let result = capture_stable(&ctx, input).await;
    db.drop().await;
    assert!(matches!(
        result,
        Err(knoted_core::files::capture::DomainError::TooLarge)
    ));
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_copy_growth_over_limit_cleans_partial() {
    let (db, ctx, root, staging, input) = fixture(8 * 1024 * 1024).await;
    let change = async {
        wait_part(&staging).await;
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("a.txt"))
            .unwrap()
            .set_len(100 * 1024 * 1024 + 1)
            .unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), change);
    db.drop().await;
    assert!(result.is_err());
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_copy_cancel_closes_and_cleans_writer() {
    let (db, ctx, _root, staging, input) = fixture(8 * 1024 * 1024).await;
    let cancel = input.cancel.clone();
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), async {
        wait_part(&staging).await;
        cancel.store(true, std::sync::atomic::Ordering::Release);
    });
    db.drop().await;
    assert!(matches!(
        result,
        Err(knoted_core::files::capture::DomainError::Cancelled)
    ));
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_abort_drops_copy_without_background_writer() {
    let (db, ctx, _root, staging, input) = fixture(8 * 1024 * 1024).await;
    let task = tokio::spawn(async move { capture_stable(&ctx, input).await });
    wait_part(&staging).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(files(&staging).is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(files(&staging).is_empty());
    db.drop().await;
}
#[tokio::test]
async fn t44_copy_path_and_root_replacement_rejected() {
    for root_changed in [false, true] {
        let (db, ctx, root, staging, input) = fixture(8 * 1024 * 1024).await;
        let offline = root.path().with_extension("offline");
        let change = async {
            wait_part(&staging).await;
            if root_changed {
                std::fs::rename(root.path(), &offline).unwrap();
                std::fs::create_dir(root.path()).unwrap();
            } else {
                std::fs::remove_file(root.path().join("a.txt")).unwrap();
                std::fs::write(root.path().join("a.txt"), "new file").unwrap();
            }
        };
        let (result, ()) = tokio::join!(capture_stable(&ctx, input), change);
        if root_changed {
            std::fs::remove_dir(root.path()).unwrap();
            std::fs::rename(&offline, root.path()).unwrap();
        }
        db.drop().await;
        assert!(result.is_err());
        assert!(files(&staging).is_empty());
    }
}
#[tokio::test]
async fn t44_staging_replacement_cannot_publish_or_write_source() {
    let (db, ctx, root, staging, input) = fixture(8 * 1024 * 1024).await;
    let offline = staging.path().with_extension("offline");
    let change = async {
        wait_part(&staging).await;
        std::fs::rename(staging.path(), &offline).unwrap();
        std::os::unix::fs::symlink(root.path(), staging.path()).unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), change);
    std::fs::remove_file(staging.path()).unwrap();
    std::fs::rename(offline, staging.path()).unwrap();
    db.drop().await;
    assert!(result.is_err());
    assert!(files(&staging).is_empty());
    assert!(!root.path().join("input").exists());
}
#[tokio::test]
async fn t44_staging_symlink_is_rejected_before_writes() {
    let (db, ctx, root, staging, input) = fixture(3).await;
    std::fs::remove_dir(staging.path()).unwrap();
    std::os::unix::fs::symlink(root.path(), staging.path()).unwrap();
    let result = capture_stable(&ctx, input).await;
    std::fs::remove_file(staging.path()).unwrap();
    std::fs::create_dir(staging.path()).unwrap();
    db.drop().await;
    assert!(result.is_err());
    assert!(!root.path().join("input").exists());
}
#[tokio::test]
async fn t44_stale_lease_cleans_closed_object() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    sqlx::query(
        "UPDATE jobs SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(input.lease.job_id)
    .execute(ctx.pool())
    .await
    .unwrap();
    let result = capture_stable(&ctx, input).await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_registry WHERE entity_type='file'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert!(result.is_err());
    assert_eq!(count, 0);
    assert!(files(&staging).is_empty());
}
async fn rescan(ctx: &AppContext, root: &TempDir, input: &SourceObservation) -> SourceObservation {
    let report = observe_root(
        ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: input.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let id = report.scan_id;
    assert!(reconcile_complete_scan(ctx, report).await.unwrap());
    knoted_core::files::capture::load_observation(ctx, id, "a.txt".into(), input.lease.clone())
        .await
        .unwrap()
}
#[tokio::test]
async fn t44_dual_versions_idempotence_and_old_object_retention() {
    let (db, ctx, root, staging, input) = fixture(3).await;
    let first = capture_stable(&ctx, input.clone()).await.unwrap();
    capture_stable(&ctx, input.clone()).await.unwrap();
    let before: (uuid::Uuid, i64, i64) = sqlx::query_as(
        "SELECT id,version,content_version FROM file_sources WHERE status='present'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    std::fs::write(root.path().join("a.txt"), "bbb").unwrap();
    let next = rescan(&ctx, &root, &input).await;
    let invalidated: (i64, i64, bool) = sqlx::query_as(
        "SELECT version,content_version,content_fresh FROM file_sources WHERE status='present'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    let strong = next.metadata.identity.handle.is_some();
    capture_stable(&ctx, next.clone()).await.unwrap();
    capture_stable(&ctx, next).await.unwrap();
    let after: (uuid::Uuid, i64, i64, bool) = sqlx::query_as(
        "SELECT id,version,content_version,content_fresh FROM file_sources WHERE status='present'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    db.drop().await;
    assert_eq!((before.1, before.2), (1, 1));
    assert_eq!(invalidated, (2, 1, false));
    if strong {
        assert_eq!(after, (before.0, 3, 2, true));
    } else {
        assert_ne!(after.0, before.0);
    }
    assert_eq!(
        std::fs::read(staging.path().join("input").join(first.object_id.as_str())).unwrap(),
        b"aaa"
    );
}
#[tokio::test]
async fn t44_same_path_replacement_never_inherits_identity_or_registry() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let before: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    std::fs::write(root.path().join("replacement"), "aaa").unwrap();
    std::fs::rename(root.path().join("replacement"), root.path().join("a.txt")).unwrap();
    let next = rescan(&ctx, &root, &input).await;
    capture_stable(&ctx, next).await.unwrap();
    let rows: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id,status FROM file_sources ORDER BY status")
            .fetch_all(ctx.pool())
            .await
            .unwrap();
    let registry: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_registry WHERE entity_type='file'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (before, "missing".into()));
    assert_ne!(rows[1].0, before);
    assert_eq!(rows[1].1, "present");
    assert_eq!(registry, 2);
}
#[tokio::test]
async fn t44_injected_same_inode_different_handle_cannot_inherit() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    // Same real dev/inode/hash, incompatible stored incarnation: deterministic
    // inode reuse counterexample without relying on allocator luck.
    sqlx::query("UPDATE file_sources SET opaque_handle=$1")
        .bind(serde_json::json!({"kind":-123,"bytes":[1,2,3]}))
        .execute(ctx.pool())
        .await
        .unwrap();
    let next = rescan(&ctx, &root, &input).await;
    capture_stable(&ctx, next).await.unwrap();
    let new: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources WHERE status='present'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert_ne!(old, new);
}
#[tokio::test]
async fn t44_source_observation_cas_change_during_copy_cleans_object() {
    let (db, ctx, _root, staging, input) = fixture(8 * 1024 * 1024).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let original_files = files(&staging);
    sqlx::query("UPDATE file_sources SET content_fresh=false")
        .execute(ctx.pool())
        .await
        .unwrap();
    let mutate = async {
        wait_part(&staging).await;
        sqlx::query("UPDATE file_sources SET observation_version=observation_version+1,version=version+1,content_fresh=false").execute(ctx.pool()).await.unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), mutate);
    let fresh: bool = sqlx::query_scalar("SELECT content_fresh FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(result.is_err());
    assert!(!fresh);
    assert_eq!(files(&staging), original_files);
}
#[tokio::test]
async fn t44_old_scan_and_partial_scan_cannot_publish() {
    let (db, ctx, root, staging, input) = fixture(3).await;
    let newer = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: input.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let partial = knoted_core::files::capture::load_observation(
        &ctx,
        newer.scan_id,
        "a.txt".into(),
        input.lease.clone(),
    )
    .await;
    let stale = capture_stable(&ctx, input).await;
    db.drop().await;
    assert!(partial.is_err());
    assert!(stale.is_err());
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_lease_expiry_while_waiting_source_lock_rolls_back() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let previous = files(&staging);
    sqlx::query("UPDATE jobs SET lease_expires_at=clock_timestamp()+interval '2600 milliseconds' WHERE id=$1").bind(input.lease.job_id).execute(ctx.pool()).await.unwrap();
    let mut lock = ctx.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM file_sources FOR UPDATE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let unblock = async {
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        lock.rollback().await.unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), unblock);
    db.drop().await;
    assert!(result.is_err());
    assert_eq!(files(&staging), previous);
}
#[tokio::test]
async fn t44_staging_leaf_replacement_cannot_publish_wrong_bytes() {
    let (db, ctx, _root, staging, input) = fixture(8 * 1024 * 1024).await;
    let replace = async {
        wait_part(&staging).await;
        let part = files(&staging).pop().unwrap();
        std::fs::remove_file(&part).unwrap();
        std::fs::write(part, "replacement").unwrap();
    };
    let (result, ()) = tokio::join!(capture_stable(&ctx, input), replace);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert!(result.is_err(), "replaced staging leaf must not publish");
    assert_eq!(count, 0);
    assert!(files(&staging).is_empty());
}
#[test]
fn t44_io_failure_cleans_partial_in_isolated_process() {
    if std::env::var_os("KNOTED_T44_IO_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "t44_io_failure_cleans_partial_in_isolated_process",
                "--nocapture",
            ])
            .env("KNOTED_T44_IO_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (db, ctx, _root, staging, input) = fixture(1024 * 1024).await;
        let mut previous = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: isolated child process only; valid rlimit pointers and SIG_IGN.
        unsafe {
            assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut previous), 0);
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            assert_eq!(
                libc::setrlimit(
                    libc::RLIMIT_FSIZE,
                    &libc::rlimit {
                        rlim_cur: 128 * 1024,
                        rlim_max: previous.rlim_max
                    }
                ),
                0
            );
        }
        let result = capture_stable(&ctx, input).await;
        // SAFETY: restore this child's saved soft limit before database cleanup.
        unsafe {
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &previous), 0);
        }
        db.drop().await;
        assert!(matches!(
            result,
            Err(knoted_core::files::capture::DomainError::Io(_))
        ));
        assert!(files(&staging).is_empty());
    });
}
#[tokio::test]
async fn t44_exact_100mib_is_accepted_without_truncation() {
    let (db, ctx, _root, staging, input) = fixture(100 * 1024 * 1024).await;
    let result = capture_stable(&ctx, input).await;
    db.drop().await;
    let object = result.unwrap();
    assert_eq!(object.size, 100 * 1024 * 1024);
    assert_eq!(
        std::fs::metadata(staging.path().join("input").join(object.object_id.as_str()))
            .unwrap()
            .len(),
        100 * 1024 * 1024
    );
}
#[tokio::test]
async fn t44_disabled_capture_cannot_read_or_publish_old_root() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    let disabled = ctx.clone().with_files(FilesConfig {
        enabled: false,
        ..ctx.files().clone()
    });
    let result = capture_stable(&disabled, input).await;
    db.drop().await;
    assert!(
        result.is_err(),
        "disabled files must not capture queued observations"
    );
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_subdirectory_symlink_replacement_rejects_even_same_leaf() {
    let (db, ctx, root, staging, input) = fixture(3).await;
    std::fs::create_dir(root.path().join("dir")).unwrap();
    std::fs::rename(root.path().join("a.txt"), root.path().join("dir/a.txt")).unwrap();
    let report = observe_root(
        &ctx,
        ObserveRoot {
            path: root.path().into(),
            cancel: Arc::default(),
            lease: input.lease.clone(),
            rules: Default::default(),
        },
    )
    .await
    .unwrap();
    let id = report.scan_id;
    assert!(reconcile_complete_scan(&ctx, report).await.unwrap());
    let input =
        knoted_core::files::capture::load_observation(&ctx, id, "dir/a.txt".into(), input.lease)
            .await
            .unwrap();
    let outside = temp();
    std::fs::rename(root.path().join("dir"), outside.path().join("dir")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("dir"), root.path().join("dir")).unwrap();
    let result = capture_stable(&ctx, input).await;
    db.drop().await;
    assert!(result.is_err());
    assert!(files(&staging).is_empty());
}
#[tokio::test]
async fn t44_same_scan_handle_conflict_never_overrides_incarnation_proof() {
    let (db, ctx, _root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let before: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE file_sources SET opaque_handle=$1")
        .bind(serde_json::json!({"kind":-123,"bytes":[1,2,3]}))
        .execute(ctx.pool())
        .await
        .unwrap();
    capture_stable(&ctx, input).await.unwrap();
    let after: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM file_sources WHERE status='present'")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    db.drop().await;
    assert_ne!(before, after, "same scan is not an incarnation proof");
}
#[tokio::test]
async fn t44_no_handle_same_observation_replays_closed_receipt_without_new_identity() {
    let (db, ctx, _root, staging, mut input) = fixture(3).await;
    let original = capture_stable(&ctx, input.clone()).await.unwrap();
    let before: (uuid::Uuid, i64, i64) =
        sqlx::query_as("SELECT id,version,content_version FROM file_sources")
            .fetch_one(ctx.pool())
            .await
            .unwrap();
    // Simulate an unsupported filesystem's observation, without replacing root
    // scope or using ctime/btime as a substitute incarnation identity.
    input.metadata.identity.handle = None;
    sqlx::query(
        "UPDATE file_sources SET opaque_handle=NULL,observation=$1,captured_observation=$1",
    )
    .bind(sqlx::types::Json(&input.metadata))
    .execute(ctx.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE file_observations SET metadata=$1 WHERE scan_id=$2")
        .bind(sqlx::types::Json(&input.metadata))
        .bind(input.scan_id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let replay = capture_stable(&ctx, input).await;
    let after: (uuid::Uuid, i64, i64) = sqlx::query_as(
        "SELECT id,version,content_version FROM file_sources WHERE status='present'",
    )
    .fetch_one(ctx.pool())
    .await
    .unwrap();
    db.drop().await;
    assert_eq!(
        replay.unwrap(),
        original,
        "receipt replay does not capture again"
    );
    assert_eq!(after, before);
    assert_eq!(files(&staging).len(), 1);
}
#[tokio::test]
async fn t44_replay_requires_valid_staged_digest_and_keeps_old_object() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    let original = capture_stable(&ctx, input.clone()).await.unwrap();
    std::fs::write(
        staging
            .path()
            .join("input")
            .join(original.object_id.as_str()),
        "bad",
    )
    .unwrap();
    let next = capture_stable(&ctx, input).await.unwrap();
    db.drop().await;
    assert_ne!(next.object_id, original.object_id);
    assert_eq!(
        std::fs::read(staging.path().join("input").join(next.object_id.as_str())).unwrap(),
        b"aaa"
    );
    assert_eq!(files(&staging).len(), 2);
}
#[tokio::test]
async fn t44_no_handle_different_scan_cannot_replay_or_inherit_old_source() {
    let (db, ctx, root, _staging, input) = fixture(3).await;
    capture_stable(&ctx, input.clone()).await.unwrap();
    let old: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE file_sources SET opaque_handle=NULL")
        .execute(ctx.pool())
        .await
        .unwrap();
    let next = rescan(&ctx, &root, &input).await;
    capture_stable(&ctx, next).await.unwrap();
    let new: uuid::Uuid = sqlx::query_scalar("SELECT id FROM file_sources WHERE status='present'")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    db.drop().await;
    assert_ne!(old, new);
}
#[tokio::test]
async fn t44_receipt_cannot_follow_staged_symlink() {
    let (db, ctx, _root, staging, input) = fixture(3).await;
    let old = capture_stable(&ctx, input.clone()).await.unwrap();
    let outside = temp();
    let target = outside.path().join("original");
    let path = staging.path().join("input").join(old.object_id.as_str());
    std::fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let new = capture_stable(&ctx, input).await.unwrap();
    db.drop().await;
    assert_ne!(old.object_id, new.object_id);
    assert_eq!(std::fs::read(target).unwrap(), b"aaa");
    assert!(std::fs::symlink_metadata(path).unwrap().is_symlink());
}

#[tokio::test]
async fn t44_p1_new_stable_content_invalidates_transition_snapshot_cas() {
    async fn snapshot(ctx: &AppContext) -> (uuid::Uuid, i64, i64, String, i64, bool) {
        sqlx::query_as("SELECT id,version,content_version,sha256,size,content_fresh FROM file_sources WHERE status='present'")
            .fetch_one(ctx.pool()).await.unwrap()
    }
    let (db, ctx, root, _staging, input) = fixture(3).await;
    assert!(
        input.metadata.identity.handle.is_some(),
        "identity-preserving test requires the supported fixture filesystem"
    );
    capture_stable(&ctx, input.clone()).await.unwrap();
    let original = snapshot(&ctx).await;
    std::fs::write(root.path().join("a.txt"), "new stable content").unwrap();
    let changed = rescan(&ctx, &root, &input).await;
    let transition = snapshot(&ctx).await;
    capture_stable(&ctx, changed.clone()).await.unwrap();
    let published = snapshot(&ctx).await;
    capture_stable(&ctx, changed).await.unwrap();
    let replayed = snapshot(&ctx).await;
    // An observed timestamp change with identical bytes must not invent a new
    // content revision or an extra publication CAS increment.
    std::fs::write(root.path().join("a.txt"), "new stable content").unwrap();
    let same_content = rescan(&ctx, &root, &input).await;
    let same_transition = snapshot(&ctx).await;
    capture_stable(&ctx, same_content.clone()).await.unwrap();
    let same_published = snapshot(&ctx).await;
    // Seed a metadata-only edit; the HTTP metadata API belongs to a later card.
    sqlx::query("UPDATE file_sources SET note='metadata-only edit',version=version+1 WHERE status='present'")
        .execute(ctx.pool()).await.unwrap();
    let metadata_edit = snapshot(&ctx).await;
    capture_stable(&ctx, same_content).await.unwrap();
    let metadata_replay = snapshot(&ctx).await;
    db.drop().await;
    assert_eq!(original.0, transition.0);
    assert_eq!(transition.0, published.0);
    assert!(transition.1 > original.1);
    assert_eq!(
        (&transition.2, &transition.3, &transition.4),
        (&original.2, &original.3, &original.4)
    );
    assert!(!transition.5);
    assert!(published.5);
    assert_ne!(
        (&published.3, published.4),
        (&transition.3, transition.4),
        "publication exposes different stable content"
    );
    assert!(published.2 > transition.2);
    assert!(
        published.1 > transition.1,
        "a CAS value read alongside the old stable content must not authorize the new content snapshot"
    );
    assert_eq!(published, replayed, "receipt replay is read-only");
    assert_eq!(
        (&same_published.2, &same_published.3),
        (&published.2, &published.3)
    );
    assert_eq!(
        same_published.1, same_transition.1,
        "unchanged bytes have no additional publication revision"
    );
    assert_eq!(
        metadata_edit, metadata_replay,
        "capture preserves a metadata-only edit"
    );
    assert_eq!(
        (&metadata_replay.2, &metadata_replay.3),
        (&same_published.2, &same_published.3)
    );
}
