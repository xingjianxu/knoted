//! Transactions, validation and the sole Task DTO shared with subsequent task cards.
use super::repository as repo;
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Todo,
    InProgress,
    Done,
    Cancelled,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TaskPriority {
    #[default]
    None,
    Low,
    Medium,
    High,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateValue {
    pub date: String,
    #[serde(deserialize_with = "required_nullable")]
    pub time: Option<String>,
    pub timezone: String,
}
fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub start: Option<DateValue>,
    pub due: Option<DateValue>,
    pub version: i64,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub(crate) archive_batch_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Default)]
pub struct CreateTask {
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    /// Outer option preserves omission; an explicit null must agree with the parent.
    pub project_id: Option<Option<Uuid>>,
    pub parent_id: Option<Uuid>,
    pub start: Option<DateValue>,
    pub due: Option<DateValue>,
}
pub struct GetTask {
    pub id: Uuid,
}
#[derive(Default)]
pub struct UpdateTask {
    pub id: Uuid,
    pub expected_version: i64,
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub project_id: Option<Option<Uuid>>,
    pub parent_id: Option<Option<Uuid>>,
    pub start: Option<Option<DateValue>>,
    pub due: Option<Option<DateValue>>,
}
pub struct MoveTaskProject {
    pub id: Uuid,
    pub expected_version: i64,
    pub project_id: Option<Uuid>,
}
pub struct DeleteTask {
    pub id: Uuid,
    pub expected_version: i64,
}
#[derive(Default)]
pub struct ListTasks {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub project_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub archived: Option<bool>,
}
#[derive(Serialize)]
pub struct TaskPage {
    pub items: Vec<Task>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("task not found")]
    NotFound,
    #[error("task version or state conflict")]
    Conflict { current_version: Option<i64> },
    #[error("invalid task field: {0}")]
    InvalidField(&'static str),
    #[error("invalid task list parameters")]
    InvalidQuery,
    #[error("task persistence unavailable")]
    Database(#[from] sqlx::Error),
}
fn conflict(t: Option<&Task>) -> DomainError {
    DomainError::Conflict {
        current_version: t.map(|t| t.version),
    }
}
fn version(v: i64) -> Result<(), DomainError> {
    if v < 1 {
        Err(DomainError::InvalidField("expected_version"))
    } else {
        Ok(())
    }
}
fn validate(t: &Task) -> Result<(), DomainError> {
    if t.title.is_empty() || t.title.chars().count() > 500 || t.title.contains('\0') {
        return Err(DomainError::InvalidField("title"));
    }
    if t.description.as_ref().is_some_and(|s| s.contains('\0')) {
        return Err(DomainError::InvalidField("description"));
    }
    super::dates::validate_dates(t.start.as_ref(), t.due.as_ref())
}

async fn finish<T>(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    r: Result<T, DomainError>,
) -> Result<T, DomainError> {
    match r {
        Ok(t) => {
            tx.commit().await?;
            Ok(t)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}
async fn project_writable(
    c: &mut sqlx::PgConnection,
    id: Option<Uuid>,
    target: Option<&Task>,
    new_ref: bool,
) -> Result<(), DomainError> {
    if let Some(id) = id {
        match repo::project_state(c, id).await? {
            None if new_ref => return Err(DomainError::InvalidField("project_id")),
            Some((false, false)) => {}
            _ => return Err(conflict(target)),
        }
    }
    Ok(())
}
async fn parent_writable(
    c: &mut sqlx::PgConnection,
    id: Uuid,
    target: Option<&Task>,
    new_ref: bool,
) -> Result<Task, DomainError> {
    let p = repo::find(c, id).await?.ok_or_else(|| {
        if new_ref {
            DomainError::InvalidField("parent_id")
        } else {
            conflict(target)
        }
    })?;
    if p.archived_at.is_some() || p.deleted_at.is_some() {
        return Err(conflict(target));
    }
    project_writable(c, p.project_id, target, false).await?;
    Ok(p)
}
async fn writable(c: &mut sqlx::PgConnection, t: &Task, expected: i64) -> Result<(), DomainError> {
    if t.deleted_at.is_some() {
        return Err(DomainError::NotFound);
    }
    if t.version != expected || t.archived_at.is_some() {
        return Err(conflict(Some(t)));
    }
    project_writable(c, t.project_id, Some(t), false).await?;
    if let Some(id) = t.parent_id {
        let p = parent_writable(c, id, Some(t), false).await?;
        if p.parent_id.is_some() || p.project_id != t.project_id {
            return Err(conflict(Some(t)));
        }
    }
    Ok(())
}
pub async fn create(ctx: &AppContext, input: CreateTask) -> Result<Task, DomainError> {
    let now = ctx.now();
    let mut t = Task {
        id: Uuid::new_v4(),
        project_id: input.project_id.flatten(),
        parent_id: input.parent_id,
        title: input.title,
        description: input.description,
        status: input.status,
        priority: input.priority,
        start: input.start,
        due: input.due,
        version: 1,
        archived_at: None,
        deleted_at: None,
        archive_batch_id: None,
        created_at: now,
        updated_at: now,
    };
    validate(&t)?;
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        repo::lock_set(&mut tx, None, input.parent_id, input.project_id.flatten()).await?;
        if let Some(id) = t.parent_id {
            let p = parent_writable(&mut tx, id, None, true).await?;
            if p.parent_id.is_some() {
                return Err(DomainError::InvalidField("parent_id"));
            }
            if input.project_id.is_some_and(|v| v != p.project_id) {
                return Err(DomainError::InvalidField("project_id"));
            }
            t.project_id = p.project_id;
        }
        project_writable(&mut tx, t.project_id, None, true).await?;
        super::transitions::validate_final_state(&mut tx, &t, None).await?;
        let t = repo::insert(&mut tx, &t).await?;
        repo::event(&mut tx, &t, "created").await?;
        Ok(t)
    }
    .await;
    finish(tx, r).await
}
pub async fn get(ctx: &AppContext, input: GetTask) -> Result<Task, DomainError> {
    repo::find(&mut *ctx.pool().acquire().await?, input.id)
        .await?
        .filter(|t| t.deleted_at.is_none())
        .ok_or(DomainError::NotFound)
}
pub async fn move_project(ctx: &AppContext, input: MoveTaskProject) -> Result<Task, DomainError> {
    update(
        ctx,
        UpdateTask {
            id: input.id,
            expected_version: input.expected_version,
            project_id: Some(input.project_id),
            ..Default::default()
        },
    )
    .await
}
pub async fn update(ctx: &AppContext, input: UpdateTask) -> Result<Task, DomainError> {
    super::transitions::transition_task(ctx, input).await
}
pub(super) async fn update_command(
    ctx: &AppContext,
    input: UpdateTask,
) -> Result<Task, DomainError> {
    version(input.expected_version)?;
    if input.title.is_none()
        && input.description.is_none()
        && input.status.is_none()
        && input.priority.is_none()
        && input.project_id.is_none()
        && input.parent_id.is_none()
        && input.start.is_none()
        && input.due.is_none()
    {
        return Err(DomainError::InvalidField("patch"));
    }
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        repo::lock_set(
            &mut tx,
            Some(input.id),
            input.parent_id.flatten(),
            input.project_id.flatten(),
        )
        .await?;
        let old = repo::find(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        writable(&mut tx, &old, input.expected_version).await?;
        let mut t = old.clone();
        if let Some(v) = input.title {
            t.title = v;
        }
        if let Some(v) = input.description {
            t.description = v;
        }
        if let Some(v) = input.status {
            t.status = v;
        }
        if let Some(v) = input.priority {
            t.priority = v;
        }
        if let Some(v) = input.start {
            t.start = v;
        }
        if let Some(v) = input.due {
            t.due = v;
        }
        if let Some(v) = input.parent_id {
            t.parent_id = v;
        }
        if let Some(v) = input.project_id {
            t.project_id = v;
        }
        let children = repo::children(&mut tx, t.id).await?;
        if let Some(id) = t.parent_id {
            if id == t.id || children.iter().any(|c| c.deleted_at.is_none()) {
                return Err(DomainError::InvalidField("parent_id"));
            }
            let p = parent_writable(&mut tx, id, Some(&old), input.parent_id.is_some()).await?;
            if p.parent_id.is_some() {
                return Err(DomainError::InvalidField("parent_id"));
            }
            if input.parent_id.is_some() && input.project_id.is_none() {
                t.project_id = p.project_id;
            }
            if t.project_id != p.project_id {
                return Err(DomainError::InvalidField("project_id"));
            }
        }
        project_writable(&mut tx, t.project_id, Some(&old), true).await?;
        validate(&t)?;
        super::transitions::validate_final_state(&mut tx, &t, Some(&old)).await?;
        if t == old {
            return Ok(old);
        }
        let moving = t.project_id != old.project_id;
        // A cascade must not repair a live child's historical inheritance or
        // move it out of a readonly source project. Trash children retain the
        // explicit relationship-migration exception.
        if moving
            && children.iter().any(|c| {
                c.deleted_at.is_none()
                    && (c.archived_at.is_some() || c.project_id != old.project_id)
            })
        {
            return Err(conflict(Some(&old)));
        }
        t.updated_at = ctx.now();
        let t = repo::save(&mut tx, &t, &old).await?;
        repo::event(&mut tx, &t, "updated").await?;
        if moving {
            for child in children {
                if child.project_id != t.project_id {
                    let changed =
                        repo::move_child(&mut tx, &child, t.project_id, t.updated_at).await?;
                    repo::event(&mut tx, &changed, "updated").await?;
                }
            }
        }
        Ok(t)
    }
    .await;
    finish(tx, r).await
}
pub async fn delete(ctx: &AppContext, input: DeleteTask) -> Result<(), DomainError> {
    version(input.expected_version)?;
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        repo::lock_set(&mut tx, Some(input.id), None, None).await?;
        let old = repo::find(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        writable(&mut tx, &old, input.expected_version).await?;
        if repo::children(&mut tx, old.id)
            .await?
            .iter()
            .any(|c| c.deleted_at.is_none())
        {
            return Err(conflict(Some(&old)));
        }
        let mut t = old.clone();
        t.updated_at = ctx.now();
        t.deleted_at = Some(t.updated_at);
        let t = repo::save(&mut tx, &t, &old).await?;
        repo::event(&mut tx, &t, "deleted").await?;
        Ok(())
    }
    .await;
    finish(tx, r).await
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    created_at: DateTime<Utc>,
    id: Uuid,
    #[serde(deserialize_with = "required_nullable")]
    project_id: Option<Uuid>,
    #[serde(deserialize_with = "required_nullable")]
    parent_id: Option<Uuid>,
    #[serde(deserialize_with = "required_nullable")]
    status: Option<TaskStatus>,
    #[serde(deserialize_with = "required_nullable")]
    priority: Option<TaskPriority>,
    #[serde(deserialize_with = "required_nullable")]
    archived: Option<bool>,
}
pub async fn list(ctx: &AppContext, input: ListTasks) -> Result<TaskPage, DomainError> {
    let limit = input.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(DomainError::InvalidQuery);
    }
    let after = input
        .cursor
        .as_ref()
        .map(|s| {
            if s.len() > 2048 {
                return Err(DomainError::InvalidQuery);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(s)
                .map_err(|_| DomainError::InvalidQuery)?;
            let c: Cursor =
                serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)?;
            if c.v != 1
                || c.scope != "tasks"
                || c.project_id != input.project_id
                || c.parent_id != input.parent_id
                || c.status != input.status
                || c.priority != input.priority
                || c.archived != input.archived
            {
                return Err(DomainError::InvalidQuery);
            }
            Ok((c.created_at, c.id))
        })
        .transpose()?;
    let mut items = repo::page(&mut *ctx.pool().acquire().await?, &input, after, limit).await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|t| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    scope: "tasks".into(),
                    created_at: t.created_at,
                    id: t.id,
                    project_id: input.project_id,
                    parent_id: input.parent_id,
                    status: input.status,
                    priority: input.priority,
                    archived: input.archived,
                })
                .expect("cursor fields serialize"),
            )
        })
    } else {
        None
    };
    Ok(TaskPage { items, next_cursor })
}
