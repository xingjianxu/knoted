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
