//! Explicit, independent notes created from completed assistant messages.
use chrono::Duration;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    context::AppContext,
    notes::{
        repository as notes_repo,
        service::{self as notes, CreateNote},
    },
};

pub struct SaveAnswer {
    pub user_id: Uuid,
    pub message_id: Uuid,
    pub title: String,
    pub parent_id: Option<Uuid>,
    pub idempotency_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("invalid idempotency key")]
    BadKey,
    #[error("chat message not found")]
    NotFound,
    #[error("answer state or idempotency conflict")]
    Conflict,
    #[error(transparent)]
    Note(#[from] notes::DomainError),
    #[error("chat storage unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(sqlx::FromRow)]
struct Answer {
    session_id: Uuid,
    run_id: Option<Uuid>,
    role: String,
    body: String,
    incomplete: bool,
    state: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Citation {
    source_type: String,
    source_id: Uuid,
    source_version: i64,
    title: String,
    location: Option<String>,
    snippet: String,
}

fn text_block(kind: &str, text: &str) -> Value {
    let mut block = json!({
        "type": kind,
        "id": Uuid::new_v4(),
        "content": [{"type": "text", "text": text}],
    });
    if kind == "heading" {
        block["level"] = json!(2);
    }
    block
}

async fn answer_document(
    tx: &mut PgConnection,
    user: Uuid,
    message: Uuid,
) -> Result<String, SaveError> {
    let answer: Answer = sqlx::query_as(
        "SELECT m.session_id,m.run_id,m.role,m.body,m.incomplete,r.state
         FROM chat_messages m JOIN chat_sessions s ON s.id=m.session_id
         LEFT JOIN generation_runs r ON r.id=m.run_id AND r.assistant_message_id=m.id
         WHERE m.id=$1 AND s.user_id=$2 AND m.deleted_at IS NULL AND s.deleted_at IS NULL
         FOR SHARE OF s",
    )
    .bind(message)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(SaveError::NotFound)?;
    if answer.role != "assistant"
        || answer.incomplete
        || answer.state.as_deref() != Some("completed")
    {
        return Err(SaveError::Conflict);
    }
    let mut blocks = vec![
        text_block("paragraph", &answer.body),
        text_block(
            "paragraph",
            &format!(
                "由用户从聊天回答转存 · 会话 {} · 回答 {message}",
                answer.session_id
            ),
        ),
    ];
    let citations: Vec<Citation> = sqlx::query_as(
        "SELECT source_type,source_id,source_version,title,location,snippet
         FROM generation_citation_snapshots WHERE message_id=$1 AND run_id=$2 ORDER BY ordinal",
    )
    .bind(message)
    .bind(answer.run_id)
    .fetch_all(tx)
    .await?;
    if !citations.is_empty() {
        blocks.push(text_block("heading", "引用快照"));
    }
    for citation in citations {
        let kind = match citation.source_type.as_str() {
            "note" => "笔记",
            "task" => "事项",
            _ => "文件",
        };
        let mut label = format!(
            "{} · {} {} · 版本 {}",
            citation.title, kind, citation.source_id, citation.source_version,
        );
        if let Some(location) = citation.location {
            label.push_str(" · ");
            label.push_str(&location);
        }
        blocks.push(text_block("paragraph", &label));
        blocks.push(text_block("paragraph", &citation.snippet));
    }
    Ok(json!({"blocks": blocks}).to_string())
}

pub async fn save_answer_as_note(ctx: &AppContext, input: SaveAnswer) -> Result<Uuid, SaveError> {
    if input.idempotency_key.is_empty() || input.idempotency_key.len() > 255 {
        return Err(SaveError::BadKey);
    }
    let key = Sha256::digest(input.idempotency_key.as_bytes()).to_vec();
    let request = Sha256::digest(
        json!([input.message_id, input.title, input.parent_id])
            .to_string()
            .as_bytes(),
    )
    .to_vec();
    // Note and idempotency record share the existing global note write lock.
    // A retry cannot observe a created note without its recorded response.
    let mut tx = notes_repo::begin_write(ctx).await?;
    let result = save_with_lock(ctx, &mut tx, input, key, request).await;
    match result {
        Ok(note) => {
            tx.commit().await?;
            Ok(note)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

async fn save_with_lock(
    ctx: &AppContext,
    tx: &mut PgConnection,
    input: SaveAnswer,
    key: Vec<u8>,
    request: Vec<u8>,
) -> Result<Uuid, SaveError> {
    let now = ctx.now();
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_sha256,note_id FROM chat_note_save_keys WHERE user_id=$1 AND key_sha256=$2 AND expires_at>$3",
    )
    .bind(input.user_id)
    .bind(&key)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((digest, note)) = prior {
        return if digest == request {
            Ok(note)
        } else {
            Err(SaveError::Conflict)
        };
    }
    if input.title.trim().is_empty() {
        return Err(notes::DomainError::InvalidField("title").into());
    }
    let document_json = answer_document(tx, input.user_id, input.message_id).await?;
    let create = CreateNote {
        title: input.title,
        parent_id: input.parent_id,
        document_json,
    };
    let note = notes::create_in_transaction(ctx, tx, create.validate(ctx)?).await?;
    sqlx::query(
        "INSERT INTO chat_note_save_keys(user_id,key_sha256,request_sha256,note_id,expires_at)
         VALUES ($1,$2,$3,$4,$5) ON CONFLICT (user_id,key_sha256) DO UPDATE
         SET request_sha256=EXCLUDED.request_sha256,note_id=EXCLUDED.note_id,expires_at=EXCLUDED.expires_at",
    )
    .bind(input.user_id)
    .bind(key)
    .bind(request)
    .bind(note.id)
    .bind(now + Duration::days(7))
    .execute(tx)
    .await?;
    Ok(note.id)
}
