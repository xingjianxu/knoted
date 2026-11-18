//! Published file metadata and atomic user annotations. Source bytes are read-only.
use super::identity::FileSource;
use crate::context::AppContext;
pub use crate::links::service::{DomainError, Page};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

const PROJECTION: &str = "SELECT s.id,s.relative_path,''::text AS name,s.size,s.sha256,s.content_version,s.version,s.status,s.mime,s.note,s.created_at,s.updated_at,s.last_seen_at,ARRAY(SELECT ft.tag_id FROM file_tags ft JOIN tags t ON t.id=ft.tag_id WHERE ft.entity_id=s.id AND t.deleted_at IS NULL ORDER BY ft.tag_id) AS tag_ids FROM file_sources s";
fn named(mut source: FileSource) -> FileSource {
    source.name = source
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    source
}
pub struct ListFiles {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
pub struct GetFile {
    pub id: Uuid,
}
pub struct UpdateFile {
    pub id: Uuid,
    pub expected_version: i64,
    pub note: Option<Option<String>>,
    pub tag_ids: Option<Vec<Uuid>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    id: Uuid,
}
pub async fn list_files(
    ctx: &AppContext,
    input: ListFiles,
) -> Result<Page<FileSource>, DomainError> {
    let limit = input.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(DomainError::InvalidQuery);
    }
    let after = input
        .cursor
        .map(|raw| {
            if raw.len() > 1024 {
                return Err(DomainError::InvalidQuery);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| DomainError::InvalidQuery)?;
            let c: Cursor =
                serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)?;
            if c.v != 1 || c.scope != "files" {
                return Err(DomainError::InvalidQuery);
            }
            Ok(c.id)
        })
        .transpose()?;
    let mut items: Vec<FileSource> = sqlx::query_as(&format!(
        "{PROJECTION} WHERE ($1::uuid IS NULL OR s.id>$1) ORDER BY s.id LIMIT $2"
    ))
    .bind(after)
    .bind(i64::from(limit) + 1)
    .fetch_all(ctx.pool())
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|s| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    v: 1,
                    scope: "files".into(),
                    id: s.id,
                })
                .expect("cursor"),
            )
        })
    } else {
        None
    };
    Ok(Page {
        items: items.into_iter().map(named).collect(),
        next_cursor,
    })
}
pub async fn get_file(ctx: &AppContext, input: GetFile) -> Result<FileSource, DomainError> {
    let mut conn = ctx.pool().acquire().await?;
    read(&mut conn, input.id).await
}
async fn read(c: &mut PgConnection, id: Uuid) -> Result<FileSource, DomainError> {
    sqlx::query_as(&format!("{PROJECTION} WHERE s.id=$1"))
        .bind(id)
        .fetch_optional(c)
        .await?
        .map(named)
        .ok_or(DomainError::NotFound)
}
pub async fn update_file(ctx: &AppContext, input: UpdateFile) -> Result<FileSource, DomainError> {
    if input.expected_version < 1
        || input
            .note
            .as_ref()
            .and_then(|v| v.as_ref())
            .is_some_and(|s| s.contains('\0'))
    {
        return Err(DomainError::InvalidField);
    }
    if input.note.is_none() && input.tag_ids.is_none() {
        return Err(DomainError::InvalidQuery);
    }
    let mut tx = ctx.pool().begin().await?;
    let result = async {
        crate::projects::repository::lock_relations(&mut tx).await?;
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM file_sources WHERE id=$1 FOR UPDATE",
        ).bind(input.id).fetch_optional(&mut *tx).await?;
        if exists.is_none() {
            return Err(DomainError::NotFound);
        }
        let old = read(&mut tx, input.id).await?;
        if old.version != input.expected_version || old.status != "present" {
            return Err(DomainError::Conflict { current_version: Some(old.version) });
        }
        let tags = if let Some(mut tags) = input.tag_ids {
            tags.sort();
            tags.dedup();
            let valid: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM tags WHERE id=ANY($1) AND deleted_at IS NULL ORDER BY id FOR UPDATE",
            ).bind(&tags).fetch_all(&mut *tx).await?;
            if tags != valid { return Err(DomainError::InvalidField); }
            tags
        } else { old.tag_ids.clone() };
        let note = input.note.unwrap_or(old.note.clone());
        if tags != old.tag_ids || note != old.note {
            // Only the active set is replaced; soft-deleted historical memberships survive.
            sqlx::query("DELETE FROM file_tags ft USING tags t WHERE ft.tag_id=t.id AND ft.entity_id=$1 AND t.deleted_at IS NULL AND NOT(ft.tag_id=ANY($2))")
                .bind(input.id).bind(&tags).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO file_tags(entity_id,tag_id) SELECT $1,unnest($2::uuid[]) ON CONFLICT DO NOTHING")
                .bind(input.id).bind(&tags).execute(&mut *tx).await?;
            sqlx::query("UPDATE file_sources SET note=$2,version=version+1,updated_at=$3 WHERE id=$1")
                .bind(input.id).bind(note).bind(ctx.now()).execute(&mut *tx).await?;
        }
        read(&mut tx, input.id).await
    }.await;
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
