use knoted_core::{
    context::AppContext,
    db::testing::TestDb,
    files::scanner::{ObserveRoot, observe_root, reconcile_complete_scan},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tempfile::TempDir;

fn root() -> TempDir {
    tempfile::tempdir_in("/data/knoted-dev/tasks/t43/tmp").unwrap()
}
async fn scan(
    ctx: &AppContext,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
) -> knoted_core::files::scanner::ScanReport {
    observe_root(
        ctx,
        ObserveRoot {
            path,
            cancel,
            lease: lease(ctx).await,
            rules: Default::default(),
        },
    )
    .await
    .unwrap()
}
async fn lease(ctx: &AppContext) -> knoted_core::jobs::Lease {
    let mut tx = ctx.pool().begin().await.unwrap();
    knoted_core::jobs::enqueue(&mut tx, knoted_core::jobs::JobSpec::new("file.scan"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    knoted_core::jobs::claim(ctx.pool(), "t43")
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn t43_1_root_loss_keeps_existing_observations() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::write(tmp.path().join("a.txt"), "original").unwrap();
    let report = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(report.complete(), "a readable root must complete");
    assert_eq!(report.paths(), vec!["a.txt"]);
    assert!(reconcile_complete_scan(&ctx, report).await.unwrap());
    std::fs::rename(tmp.path(), tmp.path().with_extension("offline")).unwrap();
    let report = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(!report.complete());
    assert!(!reconcile_complete_scan(&ctx, report).await.unwrap());
    std::fs::rename(tmp.path().with_extension("offline"), tmp.path()).unwrap();
    db.drop().await;
}
#[tokio::test]
async fn t43_2_cancelled_scan_cannot_complete() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    let cancel = Arc::new(AtomicBool::new(true));
    let report = scan(&ctx, tmp.path().into(), cancel.clone()).await;
    assert!(!report.complete());
    assert!(!reconcile_complete_scan(&ctx, report).await.unwrap());
    cancel.store(false, Ordering::Release);
    assert!(scan(&ctx, tmp.path().into(), cancel).await.complete());
    db.drop().await;
}
#[tokio::test]
async fn t43_3_symlinks_hidden_tools_and_special_files_are_excluded() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    let outside = root();
    std::fs::write(outside.path().join("secret"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("link.txt")).unwrap();
    for dir in [".git", "node_modules", "target", "cache", ".hidden"] {
        std::fs::create_dir(tmp.path().join(dir)).unwrap();
        std::fs::write(tmp.path().join(dir).join("skip.txt"), "skip").unwrap();
    }
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        tmp.path().join("pipe"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(tmp.path().join("socket")).unwrap();
    for name in [".secret", "draft.tmp", "draft~", "ok.txt"] {
        std::fs::write(tmp.path().join(name), "text").unwrap();
    }
    let report = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(report.complete());
    assert_eq!(report.paths(), vec!["ok.txt"]);
    db.drop().await;
}

async fn seed(ctx: &AppContext, scan_id: uuid::Uuid, path: &str) -> uuid::Uuid {
    use sha2::Digest;
    let configured: String = sqlx::query_scalar("SELECT configured_path FROM file_roots")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    let digest = format!(
        "{:x}",
        sha2::Sha256::digest(std::fs::read(std::path::Path::new(&configured).join(path)).unwrap())
    );
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,sha256,content_version,version,status,observation,content_fresh,last_seen_at,last_seen_scan_id) SELECT $1,s.root_id,o.relative_path,(o.metadata->'identity'->>'device')::bigint,(o.metadata->'identity'->>'inode')::bigint,(o.metadata->>'size')::bigint,(o.metadata->>'mtime_ns')::bigint,(o.metadata->>'ctime_ns')::bigint,(o.metadata->>'nlink')::bigint,$4,1,1,'present',o.metadata,true,now(),s.id FROM file_scans s JOIN file_observations o ON o.scan_id=s.id WHERE s.id=$2 AND o.relative_path=$3")
        .bind(id).bind(scan_id).bind(path).bind(digest).execute(ctx.pool()).await.unwrap();
    id
}
async fn state(ctx: &AppContext, id: uuid::Uuid) -> (String, i64, i64, i64, bool) {
    sqlx::query_as("SELECT status,version,content_version,observation_version,content_fresh FROM file_sources WHERE id=$1").bind(id).fetch_one(ctx.pool()).await.unwrap()
}
#[tokio::test]
async fn t43_complete_scan_marks_only_absent_sources_and_never_creates_unhashed_sources() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    let sid = r.scan_id;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM file_sources")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let id = seed(&ctx, sid, "a.txt").await;
    std::fs::remove_file(tmp.path().join("a.txt")).unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    assert_eq!(state(&ctx, id).await, ("missing".into(), 2, 1, 1, false));
    db.drop().await;
}
#[tokio::test]
async fn t43_1_root_loss_and_replacement_preserve_registered_sources() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    let sid = r.scan_id;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    let id = seed(&ctx, sid, "a.txt").await;
    let offline = tmp.path().with_extension("offline");
    std::fs::rename(tmp.path(), &offline).unwrap();
    for replace in [false, true] {
        if replace {
            std::fs::create_dir(tmp.path()).unwrap();
        }
        let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
        assert!(!r.complete());
        assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
        assert_eq!(state(&ctx, id).await, ("present".into(), 1, 1, 1, true));
    }
    std::fs::remove_dir(tmp.path()).unwrap();
    std::fs::rename(offline, tmp.path()).unwrap();
    db.drop().await;
}
#[tokio::test]
async fn t43_root_and_subdirectory_replacement_between_observe_and_commit_rejects_missing() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(r.complete());
    std::fs::rename(tmp.path().join("sub"), tmp.path().join("old")).unwrap();
    std::os::unix::fs::symlink("/", tmp.path().join("sub")).unwrap();
    assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(r.complete());
    let offline = tmp.path().with_extension("offline");
    std::fs::rename(tmp.path(), &offline).unwrap();
    std::fs::create_dir(tmp.path()).unwrap();
    assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
    std::fs::remove_dir(tmp.path()).unwrap();
    std::fs::rename(offline, tmp.path()).unwrap();
    db.drop().await;
}
#[tokio::test]
async fn t43_late_success_and_late_failure_cannot_replace_new_generation() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    for cancel in [false, true] {
        let old = scan(&ctx, tmp.path().into(), Arc::new(AtomicBool::new(cancel))).await;
        let new = scan(&ctx, tmp.path().into(), Arc::default()).await;
        let generation = new.generation;
        assert!(reconcile_complete_scan(&ctx, new).await.unwrap());
        assert!(matches!(
            reconcile_complete_scan(&ctx, old).await,
            Err(knoted_core::files::scanner::DomainError::Stale)
        ));
        let saved: (i64, Option<String>) =
            sqlx::query_as("SELECT last_complete_generation,paused_reason FROM file_roots")
                .fetch_one(ctx.pool())
                .await
                .unwrap();
        assert_eq!(saved, (generation, None));
    }
    db.drop().await;
}
#[tokio::test]
async fn t43_cancel_and_expired_job_fence_publication() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    let cancel = Arc::new(AtomicBool::new(false));
    let r = scan(&ctx, tmp.path().into(), cancel.clone()).await;
    cancel.store(true, Ordering::Release);
    assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    sqlx::query("UPDATE jobs SET lease_expires_at=now()-interval '1 second' WHERE id=(SELECT job_id FROM file_scans WHERE id=$1)").bind(r.scan_id).execute(ctx.pool()).await.unwrap();
    assert!(matches!(
        reconcile_complete_scan(&ctx, r).await,
        Err(knoted_core::files::scanner::DomainError::Stale)
    ));
    db.drop().await;
}
#[tokio::test]
async fn t43_changed_observation_invalidates_once_before_capture_even_if_scan_cancelled() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    let sid = r.scan_id;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    let id = seed(&ctx, sid, "a.txt").await;
    std::fs::write(tmp.path().join("a.txt"), "changed bytes").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let r = scan(&ctx, tmp.path().into(), cancel.clone()).await;
    cancel.store(true, Ordering::Release);
    assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
    assert_eq!(state(&ctx, id).await, ("present".into(), 2, 1, 2, false));
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    assert_eq!(state(&ctx, id).await, ("present".into(), 2, 1, 2, false));
    db.drop().await;
}
#[tokio::test]
async fn t43_permission_failure_is_incomplete_and_include_cannot_escape() {
    use std::os::unix::fs::PermissionsExt;
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::create_dir(tmp.path().join("locked")).unwrap();
    std::fs::set_permissions(
        tmp.path().join("locked"),
        std::fs::Permissions::from_mode(0o0),
    )
    .unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    std::fs::set_permissions(
        tmp.path().join("locked"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert!(!r.complete());
    assert!(!reconcile_complete_scan(&ctx, r).await.unwrap());
    std::fs::write(tmp.path().join(".allow"), "safe").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", tmp.path().join(".link")).unwrap();
    let input = ObserveRoot {
        path: tmp.path().into(),
        cancel: Arc::default(),
        lease: lease(&ctx).await,
        rules: knoted_core::files::scanner::ScanRules {
            include_hidden: vec![".allow".into(), ".link".into()],
            exclude: vec![],
        },
    };
    let r = observe_root(&ctx, input).await.unwrap();
    assert_eq!(r.paths(), vec![".allow"]);
    db.drop().await;
}
#[test]
fn t43_explicit_files_configuration_validates_root_and_rules() {
    let tmp = root();
    let path = tmp.path().join("config.toml");
    std::fs::write(&path,"[files]\nenabled=true\nroot='/approved/root'\nstaging_root='/private/staging'\ninclude_hidden=['.document']\nexclude=['cache']\n").unwrap();
    assert!(
        knoted_core::config::AppConfig::load(&path).is_ok(),
        "explicit single-root configuration must load"
    );
    for contents in [
        "[files]\nenabled=true",
        "[files]\nenabled=true\nroot='relative'\nstaging_root='/private'",
        "[files]\ninclude_hidden=['../escape']",
        "[files]\ndebounce_seconds=0",
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(knoted_core::config::AppConfig::load(&path).is_err());
    }
}
#[tokio::test]
async fn t43_dashboard_counts_registered_present_sources() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
    let r = scan(&ctx, tmp.path().into(), Arc::default()).await;
    let sid = r.scan_id;
    assert!(reconcile_complete_scan(&ctx, r).await.unwrap());
    let id = seed(&ctx, sid, "a.txt").await;
    let d = knoted_core::dashboard::summary(&ctx, knoted_core::dashboard::GetDashboard)
        .await
        .unwrap();
    assert_eq!(d.counts.files, 1);
    sqlx::query("UPDATE file_sources SET status='missing' WHERE id=$1")
        .bind(id)
        .execute(ctx.pool())
        .await
        .unwrap();
    let d = knoted_core::dashboard::summary(&ctx, knoted_core::dashboard::GetDashboard)
        .await
        .unwrap();
    assert_eq!(d.counts.files, 0);
    db.drop().await;
}
#[test]
fn t43_staging_alias_must_not_write_inside_readonly_source() {
    let tmp = root();
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let config = tmp.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[files]\nenabled=true\nroot='{}'\nstaging_root='{}'\n",
            source.display(),
            alias.join("staging").display()
        ),
    )
    .unwrap();
    assert!(
        knoted_core::config::AppConfig::load(&config).is_err(),
        "a symlink alias into sources must be rejected even if staging does not exist"
    );
}

#[test]
fn t43_staging_dangling_alias_into_future_root_is_rejected() {
    let tmp = root();
    let source = tmp.path().join("future-source");
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let config = tmp.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[files]\nenabled=true\nroot='{}'\nstaging_root='{}'\n",
            source.display(),
            alias.join("staging").display()
        ),
    )
    .unwrap();
    assert!(knoted_core::config::AppConfig::load(&config).is_err());
}

#[tokio::test]
async fn t43_lease_expiring_during_root_lock_wait_cannot_publish() {
    use std::time::Duration;
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let tmp = root();
    let report = scan(&ctx, tmp.path().into(), Arc::default()).await;
    sqlx::query("UPDATE jobs SET lease_expires_at=clock_timestamp()+interval '1 second' WHERE id=(SELECT job_id FROM file_scans WHERE id=$1)").bind(report.scan_id).execute(ctx.pool()).await.unwrap();
    let mut lock = ctx.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM file_roots FOR UPDATE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let ctx2 = ctx.clone();
    let pending = tokio::spawn(async move { reconcile_complete_scan(&ctx2, report).await });
    tokio::time::sleep(Duration::from_millis(1200)).await;
    lock.commit().await.unwrap();
    let result = pending.await.unwrap();
    db.drop().await;
    assert!(
        matches!(result, Err(knoted_core::files::scanner::DomainError::Stale)),
        "a lease checked before waiting is not sufficient: {result:?}"
    );
}
