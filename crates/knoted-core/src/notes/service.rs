//! Current note CRUD. Validation precedes I/O; each write owns one transaction.
use super::{
    document::{self, ValidateDocument},
    repository as repo,
};
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub document: Value,
    pub format_version: i32,
    pub content_version: i64,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

pub struct CreateNote {
    pub title: String,
    pub parent_id: Option<Uuid>,
    pub document_json: String,
}
pub struct GetNote {
    pub id: Uuid,
}
pub struct UpdateNote {
    pub id: Uuid,
    pub expected_version: i64,
    pub title: Option<String>,
    pub document_json: Option<String>,
}
pub struct DeleteNote {
    pub id: Uuid,
    pub expected_version: i64,
}
#[derive(Default)]
pub struct ListNotes {
    pub parent_id: Option<Uuid>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
#[derive(Serialize)]
pub struct NotePage {
    pub items: Vec<Note>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("note not found")]
    NotFound,
    #[error("note version or state conflict")]
    Conflict { current_version: Option<i64> },
    #[error("invalid note field: {0}")]
    InvalidField(&'static str),
    #[error("invalid note list parameters")]
    InvalidQuery,
    #[error(transparent)]
    Document(#[from] document::DomainError),
    #[error("note persistence unavailable")]
    Database(#[from] sqlx::Error),
}

fn title(value: &str) -> Result<(), DomainError> {
    if value.chars().count() > 500 {
        return Err(DomainError::InvalidField("title"));
    }
    Ok(())
}
fn version(value: i64) -> Result<(), DomainError> {
    if value < 1 {
        return Err(DomainError::InvalidField("expected_version"));
    }
    Ok(())
}
fn validate(ctx: &AppContext, raw: String) -> Result<Value, DomainError> {
    let validated = document::validate_document(
        ctx,
        ValidateDocument {
            format_version: 1,
            document_json: raw,
        },
    )?;
    // T22 validated the raw representation before JSONB canonicalization.
    serde_json::from_str(validated.document_json())
        .map_err(|_| DomainError::InvalidField("document"))
}
fn check_version(note: &Note, expected: i64) -> Result<(), DomainError> {
    if note.content_version != expected {
        return Err(DomainError::Conflict {
            current_version: Some(note.content_version),
        });
    }
    Ok(())
}

pub async fn create(ctx: &AppContext, input: CreateNote) -> Result<Note, DomainError> {
    title(&input.title)?;
    let document = validate(ctx, input.document_json)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_tree(&mut tx).await?;
        if let Some(parent) = input.parent_id
            && repo::find(&mut tx, parent, false).await?.is_none()
        {
            return Err(DomainError::Conflict {
                current_version: None,
            });
        }
        let now = ctx.now();
        let note = Note {
            id: Uuid::new_v4(),
            parent_id: input.parent_id,
            title: input.title,
            document,
            format_version: 1,
            content_version: 1,
            position: repo::next_position(&mut tx, input.parent_id).await?,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        let note = repo::insert(&mut tx, &note).await?;
        repo::event(&mut tx, note.id, 1, "created", now).await?;
        Ok(note)
    }
    .await;
    finish(tx, result).await
}

pub async fn get(ctx: &AppContext, input: GetNote) -> Result<Note, DomainError> {
    repo::find(&mut *ctx.pool().acquire().await?, input.id, false)
        .await?
        .ok_or(DomainError::NotFound)
}

pub async fn update(ctx: &AppContext, input: UpdateNote) -> Result<Note, DomainError> {
    version(input.expected_version)?;
    if input.title.is_none() && input.document_json.is_none() {
        return Err(DomainError::InvalidField("title/document"));
    }
    if let Some(t) = &input.title {
        title(t)?;
    }
    let document = input.document_json.map(|d| validate(ctx, d)).transpose()?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        let mut note = repo::find(&mut tx, input.id, true)
            .await?
            .ok_or(DomainError::NotFound)?;
        check_version(&note, input.expected_version)?;
        let changed = input.title.as_ref().is_some_and(|t| t != &note.title)
            || document.as_ref().is_some_and(|d| d != &note.document);
        if changed {
            if let Some(t) = input.title {
                note.title = t;
            }
            if let Some(d) = document {
                note.document = d;
            }
            note.updated_at = ctx.now();
            note = repo::save(&mut tx, &note, input.expected_version).await?;
            repo::event(
                &mut tx,
                note.id,
                note.content_version,
                "updated",
                note.updated_at,
            )
            .await?;
        }
        Ok(note)
    }
    .await;
    finish(tx, result).await
}

pub async fn delete(ctx: &AppContext, input: DeleteNote) -> Result<(), DomainError> {
    version(input.expected_version)?;
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        repo::lock_tree(&mut tx).await?;
        let note = repo::find(&mut tx, input.id, true)
            .await?
            .ok_or(DomainError::NotFound)?;
        check_version(&note, input.expected_version)?;
        let now = ctx.now();
        repo::soft_delete(&mut tx, &note, now).await?;
        repo::event(&mut tx, note.id, note.content_version + 1, "deleted", now).await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}

// Await cleanup before responding: a rejected parent/version must release locks
// even if the caller performs no later database operation on this connection.
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    position: i64,
    id: Uuid,
    #[serde(deserialize_with = "required_parent")]
    parent_id: Option<Uuid>,
}

fn required_parent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(d)
}

pub async fn list(ctx: &AppContext, input: ListNotes) -> Result<NotePage, DomainError> {
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
            if c.v != 1 || c.position < 0 || c.parent_id != input.parent_id {
                return Err(DomainError::InvalidQuery);
            }
            Ok(c)
        })
        .transpose()?;
    let mut items = repo::page(
        &mut *ctx.pool().acquire().await?,
        input.parent_id,
        cursor.map(|c| (c.position, c.id)),
        limit,
    )
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|n| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    position: n.position,
                    id: n.id,
                    parent_id: input.parent_id,
                })
                .expect("cursor fields serialize"),
            )
        })
    } else {
        None
    };
    Ok(NotePage { items, next_cursor })
}
