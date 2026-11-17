//! Project CRUD and lifecycle. Every writer follows the shared relation lock protocol.
use super::repository as repo;
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Project {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub version: i64,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub(crate) archive_batch_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
pub struct CreateProject {
    pub title: String,
    pub description: Option<String>,
}
pub struct GetProject {
    pub id: Uuid,
}
pub struct UpdateProject {
    pub id: Uuid,
    pub expected_version: i64,
    pub title: Option<String>,
    pub description: Option<Option<String>>,
}
pub struct DeleteProject {
    pub id: Uuid,
    pub expected_version: i64,
}
pub struct ArchiveProject {
    pub id: Uuid,
    pub expected_version: i64,
}
pub struct UnarchiveProject {
    pub id: Uuid,
    pub expected_version: i64,
}
pub struct RestoreProject {
    pub id: Uuid,
    pub expected_version: i64,
}
#[derive(Default)]
pub struct ListProjects {
    pub archived: Option<bool>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
#[derive(Serialize)]
pub struct ProjectPage {
    pub items: Vec<Project>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("project not found")]
    NotFound,
    #[error("project version or state conflict")]
    Conflict { current_version: Option<i64> },
    #[error("invalid project field: {0}")]
    InvalidField(&'static str),
    #[error("invalid project list parameters")]
    InvalidQuery,
    #[error("project persistence unavailable")]
    Database(#[from] sqlx::Error),
}
fn title(s: &str) -> Result<(), DomainError> {
    if s.is_empty() || s.chars().count() > 500 || s.contains('\0') {
        Err(DomainError::InvalidField("title"))
    } else {
        Ok(())
    }
}
fn description(s: Option<&str>) -> Result<(), DomainError> {
    if s.is_some_and(|s| s.contains('\0')) {
        Err(DomainError::InvalidField("description"))
    } else {
        Ok(())
    }
}
fn version(v: i64) -> Result<(), DomainError> {
    if v < 1 {
        Err(DomainError::InvalidField("expected_version"))
    } else {
        Ok(())
    }
}
fn conflict(p: &Project) -> DomainError {
    DomainError::Conflict {
        current_version: Some(p.version),
    }
}
fn check_version(p: &Project, v: i64) -> Result<(), DomainError> {
    if p.version != v {
        Err(conflict(p))
    } else {
        Ok(())
    }
}
async fn finish<T>(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    result: Result<T, DomainError>,
) -> Result<T, DomainError> {
    match result {
        Ok(v) => {
            tx.commit().await?;
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

pub async fn create(ctx: &AppContext, input: CreateProject) -> Result<Project, DomainError> {
    title(&input.title)?;
    description(input.description.as_deref())?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_relations(&mut tx).await?;
        let now = ctx.now();
        let p = Project {
            id: Uuid::new_v4(),
            title: input.title,
            description: input.description,
            version: 1,
            archived_at: None,
            deleted_at: None,
            archive_batch_id: None,
            created_at: now,
            updated_at: now,
        };
        let p = repo::insert(&mut tx, &p).await?;
        repo::event(&mut tx, "project.changed", p.id, p.version, "created", now).await?;
        Ok(p)
    }
    .await;
    finish(tx, result).await
}
pub async fn get(ctx: &AppContext, input: GetProject) -> Result<Project, DomainError> {
    repo::find(&mut *ctx.pool().acquire().await?, input.id)
        .await?
        .filter(|p| p.deleted_at.is_none())
        .ok_or(DomainError::NotFound)
}
pub async fn update(ctx: &AppContext, input: UpdateProject) -> Result<Project, DomainError> {
    version(input.expected_version)?;
    if input.title.is_none() && input.description.is_none() {
        return Err(DomainError::InvalidField("title/description"));
    }
    if let Some(t) = &input.title {
        title(t)?;
    }
    if let Some(d) = &input.description {
        description(d.as_deref())?;
    }
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_relations(&mut tx).await?;
        repo::lock_project(&mut tx, input.id, false).await?;
        let old = repo::find(&mut tx, input.id)
            .await?
            .filter(|p| p.deleted_at.is_none())
            .ok_or(DomainError::NotFound)?;
        check_version(&old, input.expected_version)?;
        if old.archived_at.is_some() {
            return Err(conflict(&old));
        }
        let mut p = old.clone();
        if let Some(t) = input.title {
            p.title = t;
        }
        if let Some(d) = input.description {
            p.description = d;
        }
        if p.title == old.title && p.description == old.description {
            return Ok(old);
        }
        p.updated_at = ctx.now();
        let p = repo::save(&mut tx, &p, &old).await?;
        repo::event(
            &mut tx,
            "project.changed",
            p.id,
            p.version,
            "updated",
            p.updated_at,
        )
        .await?;
        Ok(p)
    }
    .await;
    finish(tx, result).await
}
#[derive(Clone, Copy)]
enum Action {
    Archive,
    Unarchive,
    Delete,
    Restore,
}
pub async fn archive(ctx: &AppContext, input: ArchiveProject) -> Result<Project, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Archive).await
}
pub async fn unarchive(ctx: &AppContext, input: UnarchiveProject) -> Result<Project, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Unarchive).await
}
pub async fn restore(ctx: &AppContext, input: RestoreProject) -> Result<Project, DomainError> {
    change(ctx, input.id, input.expected_version, Action::Restore).await
}
pub async fn delete(ctx: &AppContext, input: DeleteProject) -> Result<(), DomainError> {
    change(ctx, input.id, input.expected_version, Action::Delete)
        .await
        .map(|_| ())
}
async fn change(
    ctx: &AppContext,
    id: Uuid,
    expected: i64,
    action: Action,
) -> Result<Project, DomainError> {
    version(expected)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_relations(&mut tx).await?;
        repo::lock_project(&mut tx, id, true).await?;
        let old = repo::find(&mut tx, id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if !matches!(action, Action::Restore) && old.deleted_at.is_some() {
            return Err(DomainError::NotFound);
        }
        check_version(&old, expected)?;
        let mut p = old.clone();
        let now = ctx.now();
        p.updated_at = now;
        let operation = match action {
            Action::Archive => {
                if old.archived_at.is_some() || repo::has_tasks(&mut tx, id, true).await? {
                    return Err(conflict(&old));
                }
                let batch = Uuid::new_v4();
                p.archived_at = Some(now);
                p.archive_batch_id = Some(batch);
                repo::archive_tasks(&mut tx, id, batch, now).await?;
                "archived"
            }
            Action::Unarchive => {
                if old.archived_at.is_none() {
                    return Err(conflict(&old));
                }
                if let Some(batch) = old.archive_batch_id {
                    repo::unarchive_tasks(&mut tx, id, batch, now).await?;
                }
                p.archived_at = None;
                p.archive_batch_id = None;
                "unarchived"
            }
            Action::Delete => {
                if old.archived_at.is_some() || repo::has_tasks(&mut tx, id, false).await? {
                    return Err(conflict(&old));
                }
                p.deleted_at = Some(now);
                "deleted"
            }
            Action::Restore => {
                if old.deleted_at.is_none() {
                    return Err(conflict(&old));
                }
                p.deleted_at = None;
                "restored"
            }
        };
        let p = repo::save(&mut tx, &p, &old).await?;
        repo::event(&mut tx, "project.changed", p.id, p.version, operation, now).await?;
        Ok(p)
    }
    .await;
    finish(tx, result).await
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    created_at: DateTime<Utc>,
    id: Uuid,
    #[serde(deserialize_with = "required_archived")]
    archived: Option<bool>,
}
fn required_archived<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Option::<bool>::deserialize(d)
}
pub async fn list(ctx: &AppContext, input: ListProjects) -> Result<ProjectPage, DomainError> {
    let limit = input.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(DomainError::InvalidQuery);
    }
    let after = input
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
            if c.v != 1 || c.scope != "projects" || c.archived != input.archived {
                return Err(DomainError::InvalidQuery);
            }
            Ok((c.created_at, c.id))
        })
        .transpose()?;
    let mut items = repo::page(
        &mut *ctx.pool().acquire().await?,
        input.archived,
        after,
        limit,
    )
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|p| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    scope: "projects".into(),
                    created_at: p.created_at,
                    id: p.id,
                    archived: input.archived,
                })
                .expect("cursor fields serialize"),
            )
        })
    } else {
        None
    };
    Ok(ProjectPage { items, next_cursor })
}
