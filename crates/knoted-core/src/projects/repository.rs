//! SQL only. Services own transactions and acquire the relation lock before discovery.
use super::service::{DomainError, Project};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

/// Shared admission protocol for all project/task relation, state and lifecycle writers.
/// Then discover rows without locks and acquire *all* project/task rows in UUID order.
/// Never enter note/asset services while holding these locks.
pub(crate) async fn lock_relations(c: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(33001,'projects'::regclass::oid::integer)")
        .execute(c)
        .await?;
    Ok(())
}

pub(super) async fn find(c: &mut PgConnection, id: Uuid) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM projects WHERE id=$1")
        .bind(id)
        .fetch_optional(c)
        .await
}

/// Discovery is protected by lock_relations, not by a premature project row lock.
/// T32 locks endpoints by the same cross-table UUID ordering.
pub(super) async fn lock_project(
    c: &mut PgConnection,
    id: Uuid,
    include_tasks: bool,
) -> Result<(), sqlx::Error> {
    let mut rows: Vec<(Uuid, bool)> = if include_tasks {
        sqlx::query_as("SELECT id,false FROM projects WHERE id=$1 UNION ALL SELECT id,true FROM tasks WHERE project_id=$1")
            .bind(id).fetch_all(&mut *c).await?
    } else {
        sqlx::query_as("SELECT id,false FROM projects WHERE id=$1")
            .bind(id)
            .fetch_all(&mut *c)
            .await?
    };
    rows.sort_unstable_by_key(|r| r.0);
    for (id, task) in rows {
        let sql = if task {
            "SELECT id FROM tasks WHERE id=$1 FOR UPDATE"
        } else {
            "SELECT id FROM projects WHERE id=$1 FOR UPDATE"
        };
        sqlx::query(sql).bind(id).fetch_optional(&mut *c).await?;
    }
    Ok(())
}

pub(super) async fn insert(c: &mut PgConnection, p: &Project) -> Result<Project, sqlx::Error> {
    sqlx::query("INSERT INTO entity_registry(id,entity_type,created_at) VALUES($1,'project',$2)")
        .bind(p.id)
        .bind(p.created_at)
        .execute(&mut *c)
        .await?;
    sqlx::query_as("INSERT INTO projects(id,title,description,created_at,updated_at) VALUES($1,$2,$3,$4,$4) RETURNING *")
        .bind(p.id).bind(&p.title).bind(&p.description).bind(p.created_at).fetch_one(c).await
}

pub(super) async fn save(
    c: &mut PgConnection,
    p: &Project,
    old: &Project,
) -> Result<Project, DomainError> {
    sqlx::query_as("UPDATE projects SET title=$3,description=$4,archived_at=$5,deleted_at=$6,archive_batch_id=$7,updated_at=$8,version=version+1 WHERE id=$1 AND version=$2 AND archived_at IS NOT DISTINCT FROM $9 AND deleted_at IS NOT DISTINCT FROM $10 RETURNING *")
        .bind(p.id).bind(old.version).bind(&p.title).bind(&p.description).bind(p.archived_at).bind(p.deleted_at).bind(p.archive_batch_id).bind(p.updated_at).bind(old.archived_at).bind(old.deleted_at)
        .fetch_optional(c).await?.ok_or(DomainError::Conflict{current_version:Some(old.version)})
}

pub(super) async fn has_tasks(
    c: &mut PgConnection,
    id: Uuid,
    unfinished_only: bool,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=$1 AND deleted_at IS NULL AND (NOT $2 OR status NOT IN ('done','cancelled')))")
        .bind(id).bind(unfinished_only).fetch_one(c).await
}

pub(super) async fn archive_tasks(
    c: &mut PgConnection,
    project: Uuid,
    batch: Uuid,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let changed:Vec<(Uuid,i64)>=sqlx::query_as("UPDATE tasks SET archived_at=$3,archive_batch_id=$2,version=version+1,updated_at=$3 WHERE project_id=$1 AND deleted_at IS NULL AND archived_at IS NULL AND status IN ('done','cancelled') RETURNING id,version")
        .bind(project).bind(batch).bind(now).fetch_all(&mut *c).await?;
    for (id, version) in changed {
        event(c, "task.changed", id, version, "archived", now).await?;
    }
    Ok(())
}

pub(super) async fn unarchive_tasks(
    c: &mut PgConnection,
    project: Uuid,
    batch: Uuid,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let changed:Vec<(Uuid,i64)>=sqlx::query_as("UPDATE tasks SET archived_at=NULL,archive_batch_id=NULL,version=version+1,updated_at=$3 WHERE project_id=$1 AND deleted_at IS NULL AND archive_batch_id=$2 RETURNING id,version")
        .bind(project).bind(batch).bind(now).fetch_all(&mut *c).await?;
    for (id, version) in changed {
        event(c, "task.changed", id, version, "unarchived", now).await?;
    }
    Ok(())
}

pub(super) async fn page(
    c: &mut PgConnection,
    archived: Option<bool>,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: u32,
) -> Result<Vec<Project>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM projects WHERE deleted_at IS NULL AND ($1::boolean IS NULL OR (archived_at IS NOT NULL)=$1) AND ($2::timestamptz IS NULL OR (created_at,id)>($2,$3)) ORDER BY created_at,id LIMIT $4")
        .bind(archived).bind(after.map(|a|a.0)).bind(after.map(|a|a.1)).bind(i64::from(limit)+1).fetch_all(c).await
}

/// Retain durable unpublished events; no job kind or source body is introduced.
pub(super) async fn event(
    c: &mut PgConnection,
    topic: &str,
    id: Uuid,
    version: i64,
    operation: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO outbox(topic,payload,created_at) VALUES($1,$2,$3)")
        .bind(topic)
        .bind(serde_json::json!({"source_id":id,"content_version":version,"operation":operation}))
        .bind(now)
        .execute(c)
        .await?;
    Ok(())
}
