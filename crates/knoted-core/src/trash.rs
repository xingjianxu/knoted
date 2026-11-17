//! Unified trash hints from one read-only snapshot. Commands remain domain-owned.
mod repository;
use crate::{context::AppContext, notes, tasks};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum EntityType {
    Note,
    Project,
    Task,
}
impl EntityType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Project => "project",
            Self::Task => "task",
        }
    }
}
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TrashItem {
    pub entity_type: EntityType,
    pub id: Uuid,
    pub title: String,
    pub deleted_at: DateTime<Utc>,
    pub parent_id: Option<Uuid>,
    pub version: i64,
    pub can_restore: bool,
    pub reason: Option<String>,
}
#[derive(Serialize)]
pub struct TrashPage {
    pub items: Vec<TrashItem>,
    pub next_cursor: Option<String>,
}
#[derive(Default)]
pub struct ListTrash {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub entity_type: Option<EntityType>,
}
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid trash parameters")]
    InvalidQuery,
    #[error("trash persistence unavailable")]
    Database(#[from] sqlx::Error),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    order: String,
    #[serde(deserialize_with = "required_nullable")]
    filter: Option<EntityType>,
    deleted_at: DateTime<Utc>,
    entity_type: EntityType,
    id: Uuid,
}
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
const ORDER: &str = "deleted_at_desc,type_asc,id_asc";
pub async fn list(ctx: &AppContext, input: ListTrash) -> Result<TrashPage, DomainError> {
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
                || c.scope != "trash"
                || c.order != ORDER
                || c.filter != input.entity_type
                || c.filter.is_some_and(|f| f != c.entity_type)
            {
                return Err(DomainError::InvalidQuery);
            }
            Ok(c)
        })
        .transpose()?;
    let mut tx = ctx.pool().begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let mut items = repository::page(&mut tx, input.entity_type, after.as_ref(), limit).await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    for item in &mut items {
        let reason = match item.entity_type {
            EntityType::Project => None,
            EntityType::Note => {
                let note = notes::repository::find_any_readonly(&mut tx, item.id)
                    .await?
                    .ok_or(sqlx::Error::RowNotFound)?;
                match notes::tree::validate_parent(&mut tx, &note, note.parent_id, false).await {
                    Ok(()) => None,
                    Err(notes::service::DomainError::Database(e)) => return Err(e.into()),
                    Err(_) => Some(
                        "原父笔记已删除、缺失或父链无效。请先恢复父笔记，或选择恢复到根目录。"
                            .into(),
                    ),
                }
            }
            EntityType::Task => {
                let old = tasks::repository::find(&mut tx, item.id)
                    .await?
                    .ok_or(sqlx::Error::RowNotFound)?;
                let mut candidate = old.clone();
                candidate.deleted_at = None;
                let result = tasks::service::relations_writable(&mut tx, &old).await;
                match result {
                    Err(tasks::service::DomainError::Database(e)) => return Err(e.into()),
                    Err(_) => Some(
                        "项目或父事项缺失、已删除、已归档，或父子关系不一致。请先恢复父对象并解除归档。".into(),
                    ),
                    Ok(()) => {
                        match tasks::transitions::validate_final_state(&mut tx, &candidate, Some(&old)).await {
                            Ok(()) => None,
                            Err(tasks::service::DomainError::Database(e)) => return Err(e.into()),
                            Err(_) => Some("恢复后父子事项的完成状态不一致。请先调整父事项或子事项状态。".into()),
                        }
                    }
                }
            }
        };
        item.can_restore = reason.is_none();
        item.reason = reason;
    }
    let next_cursor = if more {
        items.last().map(|t| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    scope: "trash".into(),
                    order: ORDER.into(),
                    filter: input.entity_type,
                    deleted_at: t.deleted_at,
                    entity_type: t.entity_type,
                    id: t.id,
                })
                .expect("cursor serializes"),
            )
        })
    } else {
        None
    };
    tx.commit().await?;
    Ok(TrashPage { items, next_cursor })
}
