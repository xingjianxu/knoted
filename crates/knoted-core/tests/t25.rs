//! Deterministic maintenance against PostgreSQL with an injected clock.
use chrono::{Duration, TimeZone, Utc};
use knoted_core::{
    context::{AppContext, FixedClock},
    db::testing::TestDb,
    notes::{
        history::{self, FlushSnapshot, ListSnapshots, MaintainHistory, RestoreSnapshot},
        service::{self, CreateNote, Note, UpdateNote},
    },
};
use std::sync::Arc;

struct Harness {
    db: TestDb,
    ctx: AppContext,
    clock: Arc<FixedClock>,
}
impl Harness {
    async fn new() -> Self {
        let db = TestDb::create().await.unwrap();
        let clock = Arc::new(FixedClock::new(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        ));
        let ctx = AppContext::new(db.pool.clone()).with_clock(clock.clone());
        Self { db, ctx, clock }
    }
    async fn create(&self) -> Note {
        service::create(
            &self.ctx,
            CreateNote {
                title: "initial".into(),
                parent_id: None,
                document_json: r#"{"blocks":[]}"#.into(),
            },
        )
        .await
        .unwrap()
    }
    async fn update(&self, n: &Note, title: &str) -> Note {
        service::update(
            &self.ctx,
            UpdateNote {
                id: n.id,
                expected_version: n.content_version,
                title: Some(title.into()),
                document_json: None,
            },
        )
        .await
        .unwrap()
    }
    async fn list(&self, n: &Note) -> Vec<history::Snapshot> {
        history::list_snapshots(
            &self.ctx,
            ListSnapshots {
                note_id: n.id,
                limit: Some(100),
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items
    }
    async fn tick(&self, recover: bool) -> history::MaintenanceReport {
        history::maintain_history(&self.ctx, MaintainHistory { recover })
            .await
            .unwrap()
    }
}

// Using last updated_at as the deadline would postpone history forever here.
#[tokio::test]
async fn t25_1_continuous_saves_snapshot_every_five_minutes_not_every_save() {
    let h = Harness::new().await;
    let mut n = h.create().await;
    for i in 1..=10 {
        h.clock.advance(Duration::seconds(30));
        n = h.update(&n, &i.to_string()).await;
        let report = h.tick(false).await;
        assert_eq!(report.snapshots, if i == 10 { 1 } else { 0 });
    }
    let snapshots = h.list(&n).await;
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].title, "10");
    assert_eq!(snapshots[0].content_version, 11);
    assert_eq!(snapshots[0].reason, "periodic");
    for i in 11..=20 {
        h.clock.advance(Duration::seconds(30));
        n = h.update(&n, &i.to_string()).await;
        h.tick(false).await;
    }
    // Second interval starts at the first unsnapped edit, 00:05:30.
    assert_eq!(h.list(&n).await.len(), 1);
    h.clock.advance(Duration::seconds(30));
    assert_eq!(h.tick(false).await.snapshots, 1);
    assert_eq!(h.list(&n).await.len(), 2);
    h.clock.advance(Duration::hours(1));
    assert_eq!(h.tick(false).await.snapshots, 0);
}

// A fresh context/worker must use the durable dirty state and latest document.
#[tokio::test]
async fn t25_2_crash_recovery_backfills_latest_once_before_five_minutes() {
    let h = Harness::new().await;
    let n = h.create().await;
    h.clock.advance(Duration::seconds(15));
    let n = h.update(&n, "latest").await;
    assert_eq!(h.tick(false).await.snapshots, 0);
    let restarted = AppContext::new(h.db.pool.clone()).with_clock(h.clock.clone());
    let r = history::maintain_history(&restarted, MaintainHistory { recover: true })
        .await
        .unwrap();
    assert_eq!(r.snapshots, 1);
    let s = h.list(&n).await;
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].title, "latest");
    assert_eq!(s[0].content_version, 2);
    assert_eq!(s[0].reason, "recovery");
    assert_eq!(h.tick(true).await.snapshots, 0);
    let f = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
        .await
        .unwrap();
    assert_eq!(f.snapshot_id, s[0].snapshot_id);
}

// Retention must never delete current content/current snapshot at its old age.
#[tokio::test]
async fn t25_retention_90_days_preserves_current_snapshot_and_boundary() {
    let h = Harness::new().await;
    let n = h.create().await;
    let old = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
        .await
        .unwrap();
    h.clock.advance(Duration::days(1));
    let n = h.update(&n, "day one").await;
    let boundary = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
        .await
        .unwrap();
    h.clock.advance(Duration::days(1));
    let n = h.update(&n, "current").await;
    let current = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
        .await
        .unwrap();
    h.clock.advance(Duration::days(89));
    let r = h.tick(false).await;
    assert_eq!(r.pruned, 1);
    let ids = h
        .list(&n)
        .await
        .into_iter()
        .map(|s| s.snapshot_id)
        .collect::<Vec<_>>();
    assert!(!ids.contains(&old.snapshot_id));
    assert!(ids.contains(&boundary.snapshot_id));
    assert!(ids.contains(&current.snapshot_id));
    h.clock.advance(Duration::days(100));
    assert_eq!(h.tick(false).await.pruned, 1);
    assert_eq!(h.list(&n).await.len(), 1);
    assert_eq!(h.list(&n).await[0].snapshot_id, current.snapshot_id);
    assert_eq!(
        service::get(&h.ctx, service::GetNote { id: n.id })
            .await
            .unwrap()
            .title,
        "current"
    );
}

// Racing API flush and two workers must still leave one current-version history.
#[tokio::test]
async fn t25_concurrent_maintenance_flush_and_restart_are_idempotent() {
    let h = Harness::new().await;
    let n = h.create().await;
    h.clock.advance(Duration::minutes(5));
    let (a, b, c) = tokio::join!(
        h.tick(false),
        h.tick(true),
        history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
    );
    c.unwrap();
    assert!(a.snapshots + b.snapshots <= 1);
    assert_eq!(h.list(&n).await.len(), 1);
    assert_eq!(h.tick(true).await.snapshots, 0);
}

// Restoring equal contents still increases version and must schedule new history.
#[tokio::test]
async fn t25_3_restore_equal_content_is_new_dirty_version_with_reverse_history() {
    let h = Harness::new().await;
    let n = h.create().await;
    let s = history::flush_snapshot(&h.ctx, FlushSnapshot { note_id: n.id })
        .await
        .unwrap();
    let n = history::restore_snapshot(
        &h.ctx,
        RestoreSnapshot {
            note_id: n.id,
            snapshot_id: s.snapshot_id,
            expected_version: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(n.content_version, 2);
    h.clock.advance(Duration::minutes(5));
    assert_eq!(h.tick(false).await.snapshots, 1);
    let s = h.list(&n).await;
    assert_eq!(s.len(), 2);
    assert!(s.iter().any(|s| s.content_version == 1));
    assert!(s.iter().any(|s| s.content_version == 2));
}

// Batching must visit more than one page without changing unchanged notes.
#[tokio::test]
async fn t25_backfill_processes_more_than_one_batch_and_noop_saves_stay_clean() {
    let h = Harness::new().await;
    let n = h.create().await;
    for _ in 0..104 {
        h.create().await;
    }
    assert_eq!(h.tick(true).await.snapshots, 105);
    h.update(&n, "initial").await;
    h.clock.advance(Duration::days(1));
    assert_eq!(h.tick(false).await.snapshots, 0);
}
