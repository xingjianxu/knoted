//! Server-admitted references and durable snapshots for historical messages.

use std::collections::HashSet;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{context::AppContext, links::service::Page, search::meili::SourceType};

use super::{chunking::ChunkLocation, retrieval::Evidence};

#[derive(Debug, Clone)]
pub struct CitationDraft {
    pub id: Uuid,
    pub chunk_set_id: Uuid,
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub source_version: i64,
    pub title: String,
    pub location: Option<String>,
    pub snippet: String,
}

impl CitationDraft {
    pub fn from_evidence(item: &Evidence) -> Self {
        Self {
            id: Uuid::new_v4(),
            chunk_set_id: item.chunk_set_id,
            source_type: item.source_type,
            source_id: item.source_id,
            source_version: item.source_version,
            title: item.title.clone(),
            location: item.locations.first().map(location),
            snippet: item.text.clone(),
        }
    }

    pub fn marker(&self) -> String {
        format!("[cite:{}]", self.id)
    }
}

fn location(point: &ChunkLocation) -> String {
    if let Some(sheet) = point
        .structure
        .get("sheet")
        .and_then(serde_json::Value::as_str)
    {
        if let Some(cell) = point
            .structure
            .get("cell")
            .and_then(serde_json::Value::as_str)
        {
            return format!("{sheet} · {cell}");
        }
        return sheet.to_owned();
    }
    match point.page {
        Some(page) => format!("第 {page} 页"),
        None => format!("段落 {}", point.block_ordinal.saturating_add(1)),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Validated {
    pub answer: String,
    pub ids: Vec<Uuid>,
}

/// Accepts references in server-issued syntax and in first-use order.
pub fn validate_citations(answer: &str, allowed: &[CitationDraft]) -> Validated {
    let allowed: HashSet<_> = allowed.iter().map(|citation| citation.id).collect();
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    let mut clean = String::with_capacity(answer.len());
    let mut remaining = answer;
    while let Some((prefix, reference)) = remaining.split_once("[cite:") {
        clean.push_str(prefix);
        let Some((id, suffix)) = reference.split_once(']') else {
            remaining = "";
            break;
        };
        if let Ok(id) = Uuid::parse_str(id)
            && allowed.contains(&id)
            && seen.insert(id)
        {
            valid.push(id);
        }
        remaining = suffix;
    }
    clean.push_str(remaining);
    let answer = clean.trim();
    if valid.is_empty() || answer.is_empty() {
        Validated {
            answer: "依据不足。".into(),
            ids: Vec::new(),
        }
    } else {
        Validated {
            answer: answer.into(),
            ids: valid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Current,
    Deleted,
    Archived,
    Missing,
}

pub fn source_state(
    original_version: i64,
    live_version: Option<i64>,
    deleted: bool,
    archived: bool,
    same_evidence: bool,
) -> SourceState {
    if deleted {
        SourceState::Deleted
    } else if live_version != Some(original_version) || !same_evidence {
        SourceState::Missing
    } else if archived {
        SourceState::Archived
    } else {
        SourceState::Current
    }
}

#[derive(Debug, Serialize)]
pub struct CitationSnapshot {
    pub id: Uuid,
    pub message_id: Uuid,
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub title: String,
    pub location: Option<String>,
    pub snippet: String,
    pub source_state: SourceState,
}

#[derive(Debug, thiserror::Error)]
pub enum CitationError {
    #[error("message not found")]
    NotFound,
    #[error("invalid citation identity")]
    Invalid,
    #[error("invalid citation query or cursor")]
    BadQuery,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn persist_citations(
    tx: &mut PgConnection,
    run_id: Uuid,
    assistant_message_id: Uuid,
    valid: &Validated,
    drafts: &[CitationDraft],
) -> Result<(), CitationError> {
    for (ordinal, id) in valid.ids.iter().enumerate() {
        let draft = drafts
            .iter()
            .find(|draft| draft.id == *id)
            .ok_or(CitationError::Invalid)?;
        let inserted = sqlx::query(
            "INSERT INTO generation_citation_snapshots
             (id,run_id,message_id,chunk_set_id,source_type,source_id,source_version,
              title,location,snippet,ordinal,visibility_generation)
             SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,dep.visibility_generation
             FROM generation_source_dependencies dep
             JOIN generation_runs g ON g.id=dep.run_id AND g.assistant_message_id=$3
             WHERE dep.run_id=$2 AND dep.chunk_set_id=$4 AND dep.source_type=$5
               AND dep.source_id=$6 AND dep.source_version=$7",
        )
        .bind(id)
        .bind(run_id)
        .bind(assistant_message_id)
        .bind(draft.chunk_set_id)
        .bind(draft.source_type.as_str())
        .bind(draft.source_id)
        .bind(draft.source_version)
        .bind(&draft.title)
        .bind(&draft.location)
        .bind(&draft.snippet)
        .bind(i32::try_from(ordinal).map_err(|_| CitationError::Invalid)?)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(CitationError::Invalid);
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SnapshotRow {
    id: Uuid,
    ordinal: i32,
    source_type: String,
    source_id: Uuid,
    source_version: i64,
    title: String,
    location: Option<String>,
    snippet: String,
    live_version: Option<i64>,
    deleted: bool,
    archived: bool,
    same_evidence: bool,
}

pub struct GetCitationSnapshots {
    pub user_id: Uuid,
    pub message_id: Uuid,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CitationCursor {
    v: u8,
    message_id: Uuid,
    ordinal: i32,
    id: Uuid,
}

impl CitationCursor {
    fn decode(raw: &str, message_id: Uuid) -> Result<Self, CitationError> {
        if raw.len() > 1024 {
            return Err(CitationError::BadQuery);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| CitationError::BadQuery)?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| CitationError::BadQuery)?;
        if cursor.v != 1 || cursor.message_id != message_id || cursor.ordinal < 0 {
            return Err(CitationError::BadQuery);
        }
        Ok(cursor)
    }

    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("citation cursor serializes"))
    }
}

/// Returns immutable snapshots with current source visibility for a saved answer.
pub async fn get_citation_snapshots(
    ctx: &AppContext,
    input: GetCitationSnapshots,
) -> Result<Page<CitationSnapshot>, CitationError> {
    let GetCitationSnapshots {
        user_id,
        message_id,
        cursor,
        limit,
    } = input;
    let limit = limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(CitationError::BadQuery);
    }
    let after = cursor
        .as_deref()
        .map(|cursor| CitationCursor::decode(cursor, message_id))
        .transpose()?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chat_messages m JOIN chat_sessions s ON s.id=m.session_id
           WHERE m.id=$1 AND s.user_id=$2 AND m.deleted_at IS NULL AND s.deleted_at IS NULL)",
    )
    .bind(message_id)
    .bind(user_id)
    .fetch_one(ctx.pool())
    .await?;
    if !exists {
        return Err(CitationError::NotFound);
    }
    let mut rows: Vec<SnapshotRow> = sqlx::query_as(
        "SELECT c.id,c.ordinal,c.source_type,c.source_id,c.source_version,c.title,c.location,c.snippet,
                COALESCE(n.content_version,t.version,f.content_version) AS live_version,
                (n.deleted_at IS NOT NULL OR t.deleted_at IS NOT NULL OR p.deleted_at IS NOT NULL) AS deleted,
                (t.archived_at IS NOT NULL OR p.archived_at IS NOT NULL) AS archived,
                (live.source_id IS NOT NULL AND COALESCE(v.generation,0)=c.visibility_generation) AS same_evidence
         FROM generation_citation_snapshots c
         JOIN chat_messages m ON m.id=c.message_id AND m.run_id=c.run_id
           AND m.deleted_at IS NULL AND NOT m.incomplete
         JOIN chat_sessions session ON session.id=m.session_id AND session.user_id=$2 AND session.deleted_at IS NULL
         LEFT JOIN notes n ON c.source_type='note' AND n.id=c.source_id
         LEFT JOIN file_sources f ON c.source_type='file' AND f.id=c.source_id
         LEFT JOIN tasks t ON c.source_type='task' AND t.id=c.source_id
         LEFT JOIN projects p ON p.id=t.project_id
         LEFT JOIN rag_chunk_sets chunk_set ON chunk_set.id=c.chunk_set_id
         LEFT JOIN current_rag_sources live ON live.source_type=c.source_type
             AND live.source_id=c.source_id AND live.source_version=c.source_version
             AND live.extraction_id=chunk_set.extraction_id
         LEFT JOIN rag_source_visibility v ON v.source_type=c.source_type AND v.source_id=c.source_id
         WHERE c.message_id=$1 AND ($3::integer IS NULL OR (c.ordinal,c.id)>($3,$4))
         ORDER BY c.ordinal,c.id LIMIT $5",
    )
    .bind(message_id)
    .bind(user_id)
    .bind(after.as_ref().map(|cursor| cursor.ordinal))
    .bind(after.as_ref().map(|cursor| cursor.id))
    .bind(i64::from(limit) + 1)
    .fetch_all(ctx.pool())
    .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|last| {
            CitationCursor {
                v: 1,
                message_id,
                ordinal: last.ordinal,
                id: last.id,
            }
            .encode()
        })
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|row| {
            Ok(CitationSnapshot {
                id: row.id,
                message_id,
                source_type: SourceType::parse(&row.source_type).ok_or(CitationError::Invalid)?,
                source_id: row.source_id,
                title: row.title,
                location: row.location,
                snippet: row.snippet,
                source_state: source_state(
                    row.source_version,
                    row.live_version,
                    row.deleted,
                    row.archived,
                    row.same_evidence,
                ),
            })
        })
        .collect::<Result<_, CitationError>>()?;
    Ok(Page { items, next_cursor })
}
