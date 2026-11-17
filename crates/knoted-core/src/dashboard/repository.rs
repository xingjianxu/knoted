//! SQL for the dashboard read snapshot; no domain writes.
use crate::tasks::service::{DateValue, Task, TaskPriority, TaskStatus};
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

pub(super) async fn tasks(c: &mut PgConnection) -> Result<Vec<Task>, sqlx::Error> {
    sqlx::query_as::<_, Row>("SELECT * FROM tasks WHERE deleted_at IS NULL AND archived_at IS NULL ORDER BY updated_at DESC,id DESC")
        .fetch_all(c).await.map(|rows| rows.into_iter().map(Into::into).collect())
}
pub(super) async fn recent_notes(
    c: &mut PgConnection,
) -> Result<Vec<crate::notes::service::Note>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM notes WHERE deleted_at IS NULL ORDER BY updated_at DESC,id DESC LIMIT 10",
    )
    .fetch_all(c)
    .await
}
pub(super) async fn counts(c: &mut PgConnection) -> Result<(i64, i64, i64, i64), sqlx::Error> {
    sqlx::query_as("SELECT (SELECT count(*) FROM notes WHERE deleted_at IS NULL), (SELECT count(*) FROM projects WHERE deleted_at IS NULL AND archived_at IS NULL), (SELECT count(*) FROM jobs WHERE state='failed'), (SELECT count(*) FROM file_sources WHERE status='present')").fetch_one(c).await
}
