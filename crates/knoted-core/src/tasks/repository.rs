//! SQL only; relation admission precedes discovery and global cross-table UUID locks.
use super::service::{DateValue, DomainError, ListTasks, Task, TaskPriority, TaskStatus};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, types::Json};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    project_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    title: String,
    description: Option<String>,
    status: TaskStatus,
    priority: TaskPriority,
    start: Option<Json<DateValue>>,
    due: Option<Json<DateValue>>,
    version: i64,
    archived_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    archive_batch_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
impl From<Row> for Task {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            project_id: r.project_id,
            parent_id: r.parent_id,
            title: r.title,
            description: r.description,
            status: r.status,
            priority: r.priority,
            start: r.start.map(|v| v.0),
            due: r.due.map(|v| v.0),
            version: r.version,
            archived_at: r.archived_at,
            deleted_at: r.deleted_at,
            archive_batch_id: r.archive_batch_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}
pub(super) async fn find(c: &mut PgConnection, id: Uuid) -> Result<Option<Task>, sqlx::Error> {
    sqlx::query_as::<_, Row>("SELECT * FROM tasks WHERE id=$1")
        .bind(id)
        .fetch_optional(c)
        .await
        .map(|r| r.map(Into::into))
}
pub(super) async fn children(c: &mut PgConnection, id: Uuid) -> Result<Vec<Task>, sqlx::Error> {
    sqlx::query_as::<_, Row>("SELECT * FROM tasks WHERE parent_id=$1 ORDER BY id")
        .bind(id)
        .fetch_all(c)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
}
pub(super) async fn project_state(
    c: &mut PgConnection,
    id: Uuid,
) -> Result<Option<(bool, bool)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT archived_at IS NOT NULL,deleted_at IS NOT NULL FROM projects WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(c)
    .await
}
/// Discover the old/new parents, all direct children (including trash), and each
/// involved project without row locks. The shared advisory lock keeps this set stable.
pub(super) async fn lock_set(
    c: &mut PgConnection,
    target: Option<Uuid>,
    parent: Option<Uuid>,
    project: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    let mut tasks = Vec::new();
    let mut projects = Vec::new();
    if let Some(id) = target {
        tasks.push(id);
        if let Some(t) = find(c, id).await? {
            tasks.extend(t.parent_id);
            projects.extend(t.project_id);
        }
        tasks.extend(children(c, id).await?.into_iter().map(|t| t.id));
    }
    tasks.extend(parent);
    projects.extend(project);
    tasks.sort_unstable();
    tasks.dedup();
    for id in &tasks {
        if let Some(t) = find(c, *id).await? {
            projects.extend(t.project_id);
        }
    }
    projects.sort_unstable();
    projects.dedup();
    let mut rows: Vec<_> = tasks
        .into_iter()
        .map(|id| (id, true))
        .chain(projects.into_iter().map(|id| (id, false)))
        .collect();
    rows.sort_unstable_by_key(|r| r.0);
    for (id, task) in rows {
        sqlx::query(if task {
            "SELECT id FROM tasks WHERE id=$1 FOR UPDATE"
        } else {
            "SELECT id FROM projects WHERE id=$1 FOR UPDATE"
        })
        .bind(id)
        .fetch_optional(&mut *c)
        .await?;
    }
    Ok(())
}
pub(super) async fn insert(c: &mut PgConnection, t: &Task) -> Result<Task, sqlx::Error> {
    sqlx::query("INSERT INTO entity_registry(id,entity_type,created_at) VALUES($1,'task',$2)")
        .bind(t.id)
        .bind(t.created_at)
        .execute(&mut *c)
        .await?;
    sqlx::query_as::<_, Row>(
        "INSERT INTO tasks(id,project_id,parent_id,title,description,status,priority,\"start\",due,created_at,updated_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10) RETURNING *",
    )
    .bind(t.id)
    .bind(t.project_id)
    .bind(t.parent_id)
    .bind(&t.title)
    .bind(&t.description)
    .bind(t.status)
    .bind(t.priority)
    .bind(t.start.as_ref().map(Json))
    .bind(t.due.as_ref().map(Json))
    .bind(t.created_at)
    .fetch_one(c)
    .await
    .map(Into::into)
}
pub(super) async fn save(c: &mut PgConnection, t: &Task, old: &Task) -> Result<Task, DomainError> {
    sqlx::query_as::<_, Row>(
        "UPDATE tasks SET project_id=$3,parent_id=$4,title=$5,description=$6,status=$7,priority=$8,
         \"start\"=$9,due=$10,deleted_at=$11,updated_at=$12,version=version+1
         WHERE id=$1 AND version=$2 AND archived_at IS NOT DISTINCT FROM $13
         AND deleted_at IS NOT DISTINCT FROM $14 RETURNING *",
    )
    .bind(t.id)
    .bind(old.version)
    .bind(t.project_id)
    .bind(t.parent_id)
    .bind(&t.title)
    .bind(&t.description)
    .bind(t.status)
    .bind(t.priority)
    .bind(t.start.as_ref().map(Json))
    .bind(t.due.as_ref().map(Json))
    .bind(t.deleted_at)
    .bind(t.updated_at)
    .bind(old.archived_at)
    .bind(old.deleted_at)
    .fetch_optional(c)
    .await?
    .map(Into::into)
    .ok_or(DomainError::Conflict {
        current_version: Some(old.version),
    })
}
/// Migrate only the child's project relation. In particular, a trash row keeps
/// its content, deletion/archive state, batch identity and external relations.
pub(super) async fn move_child(
    c: &mut PgConnection,
    old: &Task,
    project_id: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Task, DomainError> {
    sqlx::query_as::<_, Row>(
        "UPDATE tasks SET project_id=$3,updated_at=$4,version=version+1
         WHERE id=$1 AND version=$2 AND archived_at IS NOT DISTINCT FROM $5
         AND deleted_at IS NOT DISTINCT FROM $6 RETURNING *",
    )
    .bind(old.id)
    .bind(old.version)
    .bind(project_id)
    .bind(now)
    .bind(old.archived_at)
    .bind(old.deleted_at)
    .fetch_optional(c)
    .await?
    .map(Into::into)
    .ok_or(DomainError::Conflict {
        current_version: Some(old.version),
    })
}

pub(super) async fn event(
    c: &mut PgConnection,
    t: &Task,
    operation: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO outbox(topic,payload,created_at) VALUES('task.changed',$1,$2)")
        .bind(
            serde_json::json!({"source_id":t.id,"content_version":t.version,"operation":operation}),
        )
        .bind(t.updated_at)
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn page(
    c: &mut PgConnection,
    q: &ListTasks,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: u32,
) -> Result<Vec<Task>, sqlx::Error> {
    sqlx::query_as::<_, Row>(
        "SELECT * FROM tasks WHERE deleted_at IS NULL
         AND ($1::uuid IS NULL OR project_id=$1) AND ($2::uuid IS NULL OR parent_id=$2)
         AND ($3::text IS NULL OR status=$3) AND ($4::text IS NULL OR priority=$4)
         AND ($5::boolean IS NULL OR (archived_at IS NOT NULL)=$5)
         AND ($6::timestamptz IS NULL OR (created_at,id)>($6,$7))
         ORDER BY created_at,id LIMIT $8",
    )
    .bind(q.project_id)
    .bind(q.parent_id)
    .bind(q.status)
    .bind(q.priority)
    .bind(q.archived)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(i64::from(limit) + 1)
    .fetch_all(c)
    .await
    .map(|rows| rows.into_iter().map(Into::into).collect())
}
