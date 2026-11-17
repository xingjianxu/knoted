use super::repository;
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("entity or tag not found")]
    NotFound,
    #[error("version or state conflict")]
    Conflict,
    #[error("invalid field")]
    InvalidField,
    #[error("invalid pagination")]
    InvalidQuery,
    #[error("persistence unavailable")]
    Database(#[from] sqlx::Error),
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Note,
    File,
    Project,
    Task,
}
impl EntityType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::File => "file",
            Self::Project => "project",
            Self::Task => "task",
        }
    }
    pub(crate) fn tag_table(self) -> &'static str {
        match self {
            Self::Note => "note_tags",
            Self::File => "file_tags",
            Self::Project => "project_tags",
            Self::Task => "task_tags",
        }
    }
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EntityLink {
    pub id: Uuid,
    pub source_type: String,
    pub source_id: Uuid,
    pub target_type: String,
    pub target_id: Uuid,
    pub version: i64,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}
pub struct CreateTag {
    pub name: String,
}
pub struct UpdateTag {
    pub id: Uuid,
    pub name: String,
    pub expected_version: i64,
}
pub struct DeleteTag {
    pub id: Uuid,
    pub expected_version: i64,
}
pub struct ListTags {
    pub entity: Option<Entity>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, Copy)]
pub struct Entity {
    pub entity_type: EntityType,
    pub id: Uuid,
}
pub struct ChangeEntityTag {
    pub entity: Entity,
    pub tag_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLink {
    pub source_type: EntityType,
    pub source_id: Uuid,
    pub target_type: EntityType,
    pub target_id: Uuid,
}
pub struct ListLinks {
    pub source: Entity,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
pub struct RemoveLink {
    pub id: Uuid,
    pub expected_version: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    id: Uuid,
}
fn paging(
    limit: Option<u32>,
    cursor: Option<String>,
    scope: &str,
) -> Result<(i64, Option<Uuid>), DomainError> {
    let limit = limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(DomainError::InvalidQuery);
    }
    let after = cursor
        .map(|raw| {
            if raw.len() > 1024 {
                return Err(DomainError::InvalidQuery);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| DomainError::InvalidQuery)?;
            let c: Cursor =
                serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)?;
            if c.v != 1 || c.scope != scope {
                return Err(DomainError::InvalidQuery);
            }
            Ok(c.id)
        })
        .transpose()?;
    Ok((i64::from(limit), after))
}
fn page<T>(mut items: Vec<T>, limit: i64, scope: String, id: impl Fn(&T) -> Uuid) -> Page<T> {
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|item| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    scope,
                    id: id(item),
                })
                .expect("cursor serialization"),
            )
        })
    } else {
        None
    };
    Page { items, next_cursor }
}
fn name(raw: String) -> Result<String, DomainError> {
    let name = raw.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(DomainError::InvalidField);
    }
    Ok(name)
}
fn write_error(e: sqlx::Error) -> DomainError {
    if e.as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        DomainError::Conflict
    } else {
        e.into()
    }
}
pub async fn create_tag(ctx: &AppContext, input: CreateTag) -> Result<Tag, DomainError> {
    let name = name(input.name)?;
    repository::insert_tag(ctx.pool(), Uuid::new_v4(), name, ctx.now())
        .await
        .map_err(write_error)
}
pub async fn update_tag(ctx: &AppContext, input: UpdateTag) -> Result<Tag, DomainError> {
    let name = name(input.name)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let old = repository::tag(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if old.version != input.expected_version {
            return Err(DomainError::Conflict);
        }
        let row = repository::rename_tag(&mut tx, input.id, name, ctx.now())
            .await
            .map_err(write_error)?;
        Ok(row)
    }
    .await;
    finish(tx, result).await
}
pub async fn delete_tag(ctx: &AppContext, input: DeleteTag) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let tag = repository::tag(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if tag.version != input.expected_version {
            return Err(DomainError::Conflict);
        }
        repository::delete_tag(&mut tx, input.id, ctx.now()).await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
pub async fn list_tags(ctx: &AppContext, input: ListTags) -> Result<Page<Tag>, DomainError> {
    let scope = input
        .entity
        .map(|e| format!("tags:{}:{}", e.entity_type.as_str(), e.id))
        .unwrap_or("tags".into());
    let (limit, after) = paging(input.limit, input.cursor, &scope)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        if let Some(e) = input.entity {
            repository::visible(&mut tx, e, false).await?;
        }
        let rows = repository::tags(&mut tx, input.entity, limit + 1, after).await?;
        Ok(page(rows, limit, scope, |r| r.id))
    }
    .await;
    finish(tx, result).await
}
pub async fn assign_tag(ctx: &AppContext, input: ChangeEntityTag) -> Result<(), DomainError> {
    change_tag(ctx, input, true).await
}
pub async fn remove_tag(ctx: &AppContext, input: ChangeEntityTag) -> Result<(), DomainError> {
    change_tag(ctx, input, false).await
}
async fn change_tag(
    ctx: &AppContext,
    input: ChangeEntityTag,
    assign: bool,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repository::visible(&mut tx, input.entity, true).await?;
        repository::tag(&mut tx, input.tag_id)
            .await?
            .ok_or(DomainError::NotFound)?;
        repository::change_tag(&mut tx, input, assign).await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
pub async fn create_link(ctx: &AppContext, input: CreateLink) -> Result<EntityLink, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        // Lock endpoints in UUID order, including reciprocal concurrent operations.
        let source = Entity {
            entity_type: input.source_type,
            id: input.source_id,
        };
        let target = Entity {
            entity_type: input.target_type,
            id: input.target_id,
        };
        if source.id <= target.id {
            repository::visible(&mut tx, source, true).await?;
            repository::visible(&mut tx, target, false).await?;
        } else {
            repository::visible(&mut tx, target, false).await?;
            repository::visible(&mut tx, source, true).await?;
        }
        let link = repository::insert_link(&mut tx, input, ctx.now()).await?;
        Ok(link)
    }
    .await;
    finish(tx, result).await
}
pub async fn list_links(
    ctx: &AppContext,
    input: ListLinks,
) -> Result<Page<EntityLink>, DomainError> {
    let scope = format!(
        "links:{}:{}",
        input.source.entity_type.as_str(),
        input.source.id
    );
    let (limit, after) = paging(input.limit, input.cursor, &scope)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repository::visible(&mut tx, input.source, false).await?;
        let rows = repository::links(&mut tx, input.source.id, limit + 1, after).await?;
        Ok(page(rows, limit, scope, |r| r.id))
    }
    .await;
    finish(tx, result).await
}
pub async fn remove_link(ctx: &AppContext, input: RemoveLink) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let row = repository::link(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        let entity_type = match row.source_type.as_str() {
            "note" => EntityType::Note,
            "file" => EntityType::File,
            "project" => EntityType::Project,
            "task" => EntityType::Task,
            _ => return Err(DomainError::NotFound),
        };
        repository::visible(
            &mut tx,
            Entity {
                entity_type,
                id: row.source_id,
            },
            true,
        )
        .await?;
        // Re-read after the source lock: no link->source lock inversion with create.
        let row = repository::link(&mut tx, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if row.version != input.expected_version {
            return Err(DomainError::Conflict);
        }
        repository::remove_link(&mut tx, input.id).await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}

// Await rollback before replying; no row lock may outlive an error response.
async fn finish<T>(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    result: Result<T, DomainError>,
) -> Result<T, DomainError> {
    match result {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}
