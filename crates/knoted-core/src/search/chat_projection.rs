//! Transactional source events for complete dialogue text and soft deletes.

use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn record(
    tx: &mut Transaction<'_, Postgres>,
    message_id: Uuid,
    content_version: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO outbox(topic,payload) VALUES ('chat.changed',$1)")
        .bind(json!({"source_id":message_id,"source_version":content_version}))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn document(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    version: i64,
) -> Result<Option<super::meili::SourceDocument>, sqlx::Error> {
    use super::meili::{MAX_BODY_BYTES, SourceDocument, SourceType};
    type Row = (String, String, bool, chrono::DateTime<chrono::Utc>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT CASE WHEN m.deleted_at IS NULL AND s.deleted_at IS NULL THEN s.title
                     ELSE 'Deleted chat message' END,
                CASE WHEN m.deleted_at IS NULL AND s.deleted_at IS NULL THEN m.body ELSE '' END,
                m.deleted_at IS NOT NULL OR s.deleted_at IS NOT NULL, m.updated_at
         FROM chat_messages m JOIN chat_sessions s ON s.id=m.session_id
         WHERE m.id=$1 AND m.content_version=$2
          AND (NOT m.incomplete OR m.deleted_at IS NOT NULL OR s.deleted_at IS NOT NULL)",
    )
    .bind(id)
    .bind(version)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((title, mut body, deleted, updated_at)) = row else {
        return Ok(None);
    };
    if deleted {
        let had_projection: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM source_documents WHERE source_type='chat_message' AND source_id=$1)",
        ).bind(id).fetch_one(&mut **tx).await?;
        if !had_projection {
            return Ok(None);
        }
    }
    body.truncate(body.floor_char_boundary(MAX_BODY_BYTES.min(body.len())));
    Ok(Some(SourceDocument {
        source_type: SourceType::ChatMessage,
        source_id: id,
        source_version: version,
        title,
        body,
        project_id: None,
        tags: Vec::new(),
        archived: false,
        deleted,
        path: None,
        format: None,
        updated_at,
    }))
}
