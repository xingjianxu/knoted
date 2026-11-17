//! Changed-only snapshots. Row locks serialize with note writers; every snapshot,
//! restore and reference-lifecycle change belongs to its caller's transaction.
use super::{
    repository as repo,
    service::{self, DomainError, Note},
};
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Snapshot {
    pub snapshot_id: Uuid,
    pub note_id: Uuid,
    pub content_version: i64,
    pub title: String,
    pub document: Value,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}
pub struct FlushSnapshot {
    pub note_id: Uuid,
}
pub struct ListSnapshots {
    pub note_id: Uuid,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
pub struct RestoreSnapshot {
    pub note_id: Uuid,
    pub snapshot_id: Uuid,
    pub expected_version: i64,
}
#[derive(Serialize)]
pub struct SnapshotPage {
    pub items: Vec<Snapshot>,
    pub next_cursor: Option<String>,
}

pub async fn flush_snapshot(
    ctx: &AppContext,
    input: FlushSnapshot,
) -> Result<Snapshot, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let note = repo::find(&mut tx, input.note_id, true)
            .await?
            .ok_or(DomainError::NotFound)?;
        Ok(repo::snapshot(&mut tx, &note, "flush", ctx.now()).await?)
    }
    .await;
    service::finish(tx, result).await
}

pub async fn restore_snapshot(
    ctx: &AppContext,
    input: RestoreSnapshot,
) -> Result<Note, DomainError> {
    service::version(input.expected_version)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let mut note = repo::find(&mut tx, input.note_id, true)
            .await?
            .ok_or(DomainError::NotFound)?;
        service::check_version(&note, input.expected_version)?;
        let selected = repo::find_snapshot(&mut tx, note.id, input.snapshot_id)
            .await?
            .ok_or(DomainError::NotFound)?;
        let now = ctx.now();
        repo::snapshot(&mut tx, &note, "before_restore", now).await?;
        note.title = selected.title;
        note.document = selected.document;
        note.updated_at = now;
        // Restore is an explicit new version, even when its content is equal.
        let note = repo::save(&mut tx, &note, input.expected_version).await?;
        repo::event(
            &mut tx,
            note.id,
            note.content_version,
            "snapshot_restored",
            now,
        )
        .await?;
        Ok(note)
    }
    .await;
    service::finish(tx, result).await
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    note_id: Uuid,
    created_at: DateTime<Utc>,
    snapshot_id: Uuid,
}

pub async fn list_snapshots(
    ctx: &AppContext,
    input: ListSnapshots,
) -> Result<SnapshotPage, DomainError> {
    let limit = input.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(DomainError::InvalidQuery);
    }
    let cursor = input
        .cursor
        .map(|s| {
            if s.len() > 1024 {
                return Err(DomainError::InvalidQuery);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(s)
                .map_err(|_| DomainError::InvalidQuery)?;
            let c: Cursor =
                serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)?;
            if c.v != 1 || c.note_id != input.note_id {
                return Err(DomainError::InvalidQuery);
            }
            Ok(c)
        })
        .transpose()?;
    let mut c = ctx.pool().acquire().await?;
    repo::find(&mut c, input.note_id, false)
        .await?
        .ok_or(DomainError::NotFound)?;
    let mut items = repo::snapshot_page(
        &mut c,
        input.note_id,
        cursor.map(|c| (c.created_at, c.snapshot_id)),
        limit,
    )
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|s| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    note_id: input.note_id,
                    created_at: s.created_at,
                    snapshot_id: s.snapshot_id,
                })
                .expect("cursor fields serialize"),
            )
        })
    } else {
        None
    };
    Ok(SnapshotPage { items, next_cursor })
}

/// Startup recovery bypasses the interval once; normal passes snapshot due notes.
pub struct MaintainHistory {
    pub recover: bool,
}
#[derive(Default)]
pub struct MaintenanceReport {
    pub snapshots: u64,
    pub pruned: u64,
}
pub async fn maintain_history(
    ctx: &AppContext,
    input: MaintainHistory,
) -> Result<MaintenanceReport, DomainError> {
    let now = ctx.now();
    let due = if input.recover {
        now
    } else {
        now - Duration::minutes(5)
    };
    let reason = if input.recover {
        "recovery"
    } else {
        "periodic"
    };
    let mut report = MaintenanceReport::default();
    loop {
        let mut tx = ctx.pool().begin().await?;
        let result = async {
            let notes = repo::history_due(&mut tx, due).await?;
            let count = notes.len();
            for note in notes {
                repo::snapshot(&mut tx, &note, reason, now).await?;
            }
            Ok(count)
        }
        .await;
        let count = service::finish(tx, result).await?;
        report.snapshots += count as u64;
        if count < 100 {
            break;
        }
    }
    loop {
        let mut tx = ctx.pool().begin().await?;
        let result = repo::prune_snapshots(&mut tx, now - Duration::days(90))
            .await
            .map_err(DomainError::from);
        let (notes, pruned) = service::finish(tx, result).await?;
        report.pruned += pruned;
        if notes < 100 {
            break;
        }
    }
    Ok(report)
}
