//! Note SQL. Callers own transactions; structural writers lock the tree first.
use super::service::{DomainError, Note};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

/// Shared by creation/deletion and future move/restore operations. A global
/// transaction lock also covers the virtual root, which has no row to lock.
pub(crate) async fn lock_tree(c: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(23001, 'notes'::regclass::oid::integer)")
        .execute(c)
        .await?;
    Ok(())
}

pub(crate) async fn find(
    c: &mut PgConnection,
    id: Uuid,
    lock: bool,
) -> Result<Option<Note>, sqlx::Error> {
    let sql = if lock {
        "SELECT * FROM notes WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    } else {
        "SELECT * FROM notes WHERE id=$1 AND deleted_at IS NULL"
    };
    sqlx::query_as(sql).bind(id).fetch_optional(c).await
}

pub(crate) async fn insert(c: &mut PgConnection, note: &Note) -> Result<Note, sqlx::Error> {
    sqlx::query("INSERT INTO entity_registry (id,entity_type,created_at) VALUES ($1,'note',$2)")
        .bind(note.id)
        .bind(note.created_at)
        .execute(&mut *c)
        .await?;
    sqlx::query_as("INSERT INTO notes (id,parent_id,title,document,position,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$6) RETURNING *")
        .bind(note.id).bind(note.parent_id).bind(&note.title).bind(&note.document).bind(note.position).bind(note.created_at).fetch_one(c).await
}

pub(crate) async fn next_position(
    c: &mut PgConnection,
    parent: Option<Uuid>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(max(position)+1,0) FROM notes WHERE parent_id IS NOT DISTINCT FROM $1 AND deleted_at IS NULL").bind(parent).fetch_one(c).await
}

pub(crate) async fn save(
    c: &mut PgConnection,
    note: &Note,
    expected: i64,
) -> Result<Note, DomainError> {
    let row = sqlx::query_as("UPDATE notes SET title=$3,document=$4,content_version=content_version+1,updated_at=$5 WHERE id=$1 AND content_version=$2 AND deleted_at IS NULL RETURNING *")
        .bind(note.id).bind(expected).bind(&note.title).bind(&note.document).bind(note.updated_at).fetch_optional(c).await?;
    row.ok_or(DomainError::Conflict {
        current_version: Some(note.content_version),
    })
}

pub(crate) async fn soft_delete(
    c: &mut PgConnection,
    note: &Note,
    now: DateTime<Utc>,
) -> Result<(), DomainError> {
    let has_children: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM notes WHERE parent_id=$1 AND deleted_at IS NULL)",
    )
    .bind(note.id)
    .fetch_one(&mut *c)
    .await?;
    if has_children {
        return Err(DomainError::Conflict {
            current_version: Some(note.content_version),
        });
    }
    let changed = sqlx::query("UPDATE notes SET deleted_at=$3,updated_at=$3,content_version=content_version+1 WHERE id=$1 AND content_version=$2 AND deleted_at IS NULL")
        .bind(note.id).bind(note.content_version).bind(now).execute(&mut *c).await?;
    if changed.rows_affected() == 0 {
        return Err(DomainError::NotFound);
    }
    // Position is structural metadata. Repacking does not change sibling content.
    sqlx::query("UPDATE notes SET position=position-1 WHERE parent_id IS NOT DISTINCT FROM $1 AND deleted_at IS NULL AND position>$2")
        .bind(note.parent_id).bind(note.position).execute(c).await?;
    Ok(())
}

pub(crate) async fn page(
    c: &mut PgConnection,
    parent: Option<Uuid>,
    after: Option<(i64, Uuid)>,
    limit: u32,
) -> Result<Vec<Note>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM notes WHERE deleted_at IS NULL AND ($1::uuid IS NULL OR parent_id=$1) AND ($2::bigint IS NULL OR (position,id)>($2,$3)) ORDER BY position,id LIMIT $4")
        .bind(parent).bind(after.map(|v|v.0)).bind(after.map(|v|v.1)).bind(i64::from(limit)+1).fetch_all(c).await
}

/// No worker handler exists yet: retain an unpublished durable event without
/// enqueueing a kind the current worker would reject. No source text is copied.
pub(crate) async fn event(
    c: &mut PgConnection,
    id: Uuid,
    version: i64,
    operation: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO outbox (topic,payload,created_at) VALUES ('note.changed',$1,$2)")
        .bind(serde_json::json!({"source_id":id,"content_version":version,"operation":operation}))
        .bind(now)
        .execute(c)
        .await?;
    Ok(())
}
