use knoted_core::{context::AppContext, db::testing::TestDb, maintenance::exclusive};
use std::time::Duration;

#[tokio::test]
async fn maintenance_waits_for_writer_and_reopens() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let writer = ctx.try_write().await.unwrap().unwrap();
    let pool = ctx.admission_pool().clone();
    let backup = tokio::spawn(async move { exclusive(&pool, Duration::from_secs(5)).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(probe) = ctx.try_write().await.unwrap() {
            probe.finish().await.unwrap();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!backup.is_finished());
    sqlx::query("CREATE TABLE admitted_commit (id integer)")
        .execute(ctx.pool())
        .await
        .unwrap();
    writer.finish().await.unwrap();
    let mut guard = backup.await.unwrap().unwrap();
    guard.check().await.unwrap();
    assert!(ctx.try_write().await.unwrap().is_none());
    assert!(
        sqlx::query("SELECT id FROM admitted_commit")
            .execute(ctx.pool())
            .await
            .is_ok()
    );
    drop(guard);
    ctx.try_write()
        .await
        .unwrap()
        .unwrap()
        .finish()
        .await
        .unwrap();
    db.drop().await;
}

#[tokio::test]
async fn lost_writer_backend_leaves_marker_which_refuses_backup() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let writer = ctx.try_write().await.unwrap().unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT backend_pid FROM maintenance_operations")
        .fetch_one(ctx.pool())
        .await
        .unwrap();
    sqlx::query("SELECT pg_terminate_backend($1, 1000)")
        .bind(pid)
        .execute(ctx.pool())
        .await
        .unwrap();
    let refused = exclusive(ctx.admission_pool(), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(refused, sqlx::Error::Protocol(_)), "{refused}");
    drop(writer);
    db.drop().await;
}

#[tokio::test]
async fn timed_out_coordinator_leaves_no_waiter_and_lost_coordinator_never_reconnects() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let writer = ctx.try_write().await.unwrap().unwrap();
    assert!(
        exclusive(ctx.admission_pool(), Duration::from_millis(100))
            .await
            .is_err()
    );
    writer.finish().await.unwrap();

    let mut guard = exclusive(ctx.admission_pool(), Duration::from_secs(2))
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pid FROM pg_locks WHERE locktype='advisory' AND classid=1263423300 AND objid=current_schema()::regnamespace::oid AND mode='ExclusiveLock' AND granted")
        .fetch_one(ctx.pool()).await.unwrap();
    sqlx::query("SELECT pg_terminate_backend($1, 1000)")
        .bind(pid)
        .execute(ctx.pool())
        .await
        .unwrap();
    assert!(guard.check().await.is_err());
    drop(guard);
    ctx.try_write()
        .await
        .unwrap()
        .unwrap()
        .finish()
        .await
        .unwrap();
    db.drop().await;
}
