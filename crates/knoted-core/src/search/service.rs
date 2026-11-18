//! Unified browser search. PostgreSQL admits every candidate before disclosure.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::meili::{self, Candidate, MAX_SEARCH_HITS, MeiliClient, SourceType};
use crate::{context::AppContext, links::service::Page};

const ENGINE_BATCH: usize = 100;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    Note,
    File,
    Task,
    Project,
    Chat,
}

impl SearchType {
    fn source(self) -> SourceType {
        match self {
            Self::Note => SourceType::Note,
            Self::File => SourceType::File,
            Self::Task => SourceType::Task,
            Self::Project => SourceType::Project,
            Self::Chat => SourceType::ChatMessage,
        }
    }

    fn from_source(source: SourceType) -> Self {
        match source {
            SourceType::Note => Self::Note,
            SourceType::File => Self::File,
            SourceType::Task => Self::Task,
            SourceType::Project => Self::Project,
            SourceType::ChatMessage => Self::Chat,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    #[default]
    Relevance,
    UpdatedDesc,
    UpdatedAsc,
}

impl SearchSort {
    fn engine(self) -> &'static [&'static str] {
        match self {
            Self::Relevance => &[],
            Self::UpdatedDesc => &["updated_at:desc", "id:asc"],
            Self::UpdatedAsc => &["updated_at:asc", "id:asc"],
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchFilters {
    pub q: String,
    #[serde(rename = "type")]
    pub entity_type: Option<SearchType>,
    pub tags: Vec<Uuid>,
    pub project: Option<Uuid>,
    pub updated_from: Option<DateTime<Utc>>,
    pub updated_to: Option<DateTime<Utc>>,
    pub archived: Option<bool>,
    pub path: Option<String>,
    pub format: Option<String>,
    pub sort: SearchSort,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(flatten)]
    pub filters: SearchFilters,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub id: Uuid,
    pub entity_type: SearchType,
    pub title: String,
    pub snippet: String,
    pub updated_at: DateTime<Utc>,
    pub path: Option<String>,
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid search query or cursor")]
    InvalidQuery,
    #[error("search index unavailable")]
    Engine(#[from] meili::DomainError),
    #[error("search persistence unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    offset: usize,
}

impl Cursor {
    fn decode(raw: &str, scope: &str) -> Result<usize, DomainError> {
        if raw.len() > 1024 {
            return Err(DomainError::InvalidQuery);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| DomainError::InvalidQuery)?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)?;
        if cursor.v != 1 || cursor.scope != scope || !(1..MAX_SEARCH_HITS).contains(&cursor.offset)
        {
            return Err(DomainError::InvalidQuery);
        }
        Ok(cursor.offset)
    }

    fn encode(scope: String, offset: usize) -> String {
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Self {
                v: 1,
                scope,
                offset,
            })
            .expect("search cursor serializes"),
        )
    }
}

async fn visible(
    ctx: &AppContext,
    candidates: &[Candidate],
    filters: &SearchFilters,
) -> Result<Vec<VisibleHit>, sqlx::Error> {
    let keys: Vec<&str> = candidates.iter().map(|entry| entry.key.as_str()).collect();
    // Projection metadata can lag file moves, tagging, and relation changes
    // which leave the content version intact. Read their current values here.
    sqlx::query_as(
        r#"
        WITH tag_links AS (
            SELECT entity_id, tag_id FROM note_tags
            UNION ALL SELECT entity_id, tag_id FROM file_tags
            UNION ALL SELECT entity_id, tag_id FROM project_tags
            UNION ALL SELECT entity_id, tag_id FROM task_tags
        )
        SELECT hits.ordinality, source.source_type, source.source_id,
               live.title, live.path, source.updated_at, live.session_id
        FROM unnest($1::text[]) WITH ORDINALITY hits(document_key, ordinality)
        JOIN fulltext_publications publication ON publication.document_key=hits.document_key
          AND publication.engine='meilisearch' AND publication.index_uid=$2
          AND publication.status='published'
        JOIN source_documents source ON source.id=publication.document_id
          AND source.superseded_at IS NULL AND NOT source.deleted
        JOIN LATERAL (
            SELECT n.title, NULL::text AS path,
                   false AS archived, NULL::uuid AS project_id, NULL::uuid AS session_id
            FROM notes n
            WHERE source.source_type='note' AND n.id=source.source_id
              AND n.deleted_at IS NULL AND n.content_version=source.source_version
            UNION ALL
            SELECT regexp_replace(f.relative_path, '^.*/', ''), f.relative_path,
                   false, NULL::uuid, NULL::uuid
            FROM file_sources f
            WHERE source.source_type='file' AND f.id=source.source_id
              AND f.status='present' AND f.content_fresh
              AND f.content_version=source.source_version
            UNION ALL
            SELECT p.title, NULL::text, p.archived_at IS NOT NULL, p.id, NULL::uuid
            FROM projects p
            WHERE source.source_type='project' AND p.id=source.source_id
              AND p.deleted_at IS NULL AND p.version=source.source_version
            UNION ALL
            SELECT t.title, NULL::text, t.archived_at IS NOT NULL, t.project_id, NULL::uuid
            FROM tasks t
            WHERE source.source_type='task' AND t.id=source.source_id
              AND t.deleted_at IS NULL AND t.version=source.source_version
            UNION ALL
            SELECT s.title, NULL::text, false, NULL::uuid, s.id
            FROM chat_messages m JOIN chat_sessions s ON s.id=m.session_id
            WHERE source.source_type='chat_message' AND m.id=source.source_id
              AND m.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT m.incomplete
              AND m.content_version=source.source_version
        ) live ON true
        WHERE ($3::text IS NULL OR source.source_type=$3)
          AND ($4::timestamptz IS NULL OR source.updated_at >= $4)
          AND ($5::timestamptz IS NULL OR source.updated_at <= $5)
          AND ($6::bool IS NULL OR live.archived=$6)
          AND ($7::text IS NULL OR live.path=$7 OR starts_with(live.path, $7 || '/'))
          AND ($8::text IS NULL OR lower(substring(live.path FROM '\.([^./]+)$'))=$8)
          AND ($9::uuid IS NULL OR (EXISTS (
                  SELECT 1 FROM projects p WHERE p.id=$9 AND p.deleted_at IS NULL
              ) AND (live.project_id=$9 OR EXISTS (
                  SELECT 1 FROM entity_links l
                  WHERE (l.source_id=source.source_id AND l.target_id=$9)
                     OR (l.source_id=$9 AND l.target_id=source.source_id)
              ))))
          AND NOT EXISTS (
              SELECT 1 FROM unnest($10::uuid[]) requested(tag_id)
              WHERE NOT EXISTS (
                  SELECT 1 FROM tag_links link JOIN tags tag ON tag.id=link.tag_id
                  WHERE link.entity_id=source.source_id AND link.tag_id=requested.tag_id
                    AND tag.deleted_at IS NULL
              )
          )
        ORDER BY hits.ordinality
        "#,
    )
    .bind(keys)
    .bind(&ctx.search().index_uid)
    .bind(filters.entity_type.map(|kind| kind.source().as_str()))
    .bind(filters.updated_from)
    .bind(filters.updated_to)
    .bind(filters.archived)
    .bind(&filters.path)
    .bind(&filters.format)
    .bind(filters.project)
    .bind(&filters.tags)
    .fetch_all(ctx.pool())
    .await
}

#[derive(sqlx::FromRow)]
struct VisibleHit {
    ordinality: i64,
    source_type: String,
    source_id: Uuid,
    title: String,
    path: Option<String>,
    updated_at: DateTime<Utc>,
    session_id: Option<Uuid>,
}

/// Searches the active document index, with a query-bound engine offset.
pub async fn search(ctx: &AppContext, input: SearchQuery) -> Result<Page<SearchHit>, DomainError> {
    let SearchQuery {
        mut filters,
        limit,
        cursor,
    } = input;
    let limit = limit.unwrap_or(50);
    filters.q = filters.q.trim().to_owned();
    if filters.q.is_empty()
        || filters.q.chars().count() > 500
        || filters.q.contains('\0')
        || !(1..=100).contains(&limit)
        || filters.tags.len() > 100
        || filters
            .updated_from
            .zip(filters.updated_to)
            .is_some_and(|(from, to)| from > to)
        || filters
            .path
            .as_ref()
            .is_some_and(|value| value.len() > 4096 || value.contains('\0'))
        || filters
            .format
            .as_ref()
            .is_some_and(|value| value.len() > 32 || value.contains('\0'))
    {
        return Err(DomainError::InvalidQuery);
    }
    filters.tags.sort_unstable();
    filters.tags.dedup();
    filters.path = filters
        .path
        .map(|path| path.trim_end_matches('/').to_owned())
        .filter(|path| !path.is_empty());
    filters.format = filters.format.map(|format| format.to_ascii_lowercase());
    let filter_bytes =
        serde_json::to_vec(&(&ctx.search().index_uid, &filters)).expect("search scope serializes");
    let scope = format!("{:x}", Sha256::digest(&filter_bytes));
    let mut offset = cursor
        .as_deref()
        .map(|cursor| Cursor::decode(cursor, &scope))
        .transpose()?
        .unwrap_or(0);

    let mut result = Page {
        items: Vec::new(),
        next_cursor: None,
    };
    let client = MeiliClient::from_context(ctx)?;
    let engine_filter = match filters.entity_type {
        Some(kind) => format!(
            "deleted = false AND source_type = \"{}\"",
            kind.source().as_str()
        ),
        None => {
            "deleted = false AND source_type IN [file, note, project, task, chat_message]".into()
        }
    };
    while offset < MAX_SEARCH_HITS {
        let count = ENGINE_BATCH.min(MAX_SEARCH_HITS - offset);
        let candidates = client
            .search_candidates(
                &ctx.search().index_uid,
                &filters.q,
                &engine_filter,
                filters.sort.engine(),
                offset,
                count,
            )
            .await?;
        if candidates.is_empty() {
            break;
        }
        for entry in visible(ctx, &candidates, &filters).await? {
            let Some(kind) = SourceType::parse(&entry.source_type) else {
                continue;
            };
            let index = (entry.ordinality - 1) as usize;
            if result.items.len() == limit {
                result.next_cursor = Some(Cursor::encode(scope, offset + index));
                return Ok(result);
            }
            let candidate = &candidates[index];
            result.items.push(SearchHit {
                id: entry.source_id,
                entity_type: SearchType::from_source(kind),
                title: entry.title,
                snippet: candidate.formatted.body.chars().take(600).collect(),
                updated_at: entry.updated_at,
                path: entry.path,
                score: candidate.score,
                session_id: entry.session_id,
            });
        }
        if candidates.len() < count {
            break;
        }
        offset += count;
    }
    Ok(result)
}

const PROJECTION_BATCH: usize = 32;

#[derive(Deserialize)]
struct SourceEvent {
    source_id: Uuid,
    #[serde(alias = "content_version")]
    source_version: i64,
    extraction_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ProjectionRow {
    title: String,
    body: String,
    document: Option<serde_json::Value>,
    project_id: Option<Uuid>,
    archived: bool,
    deleted: bool,
    path: Option<String>,
    updated_at: DateTime<Utc>,
}

async fn source_document(
    ctx: &AppContext,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: SourceType,
    event: SourceEvent,
) -> Result<Option<meili::SourceDocument>, sqlx::Error> {
    use crate::notes::document::{
        ExtractPlainText, ValidateDocument, extract_plain_text, validate_document,
    };

    let sql = match source {
        SourceType::Note => {
            "SELECT title, '' AS body, CASE WHEN deleted_at IS NULL THEN document END AS document,
                    NULL::uuid AS project_id, false AS archived, deleted_at IS NOT NULL AS deleted,
                    NULL::text AS path, updated_at
             FROM notes WHERE id=$1 AND content_version=$2"
        }
        SourceType::Project => {
            "SELECT title, CASE WHEN deleted_at IS NULL THEN COALESCE(description, '') ELSE '' END AS body,
                    NULL::jsonb AS document, id AS project_id, archived_at IS NOT NULL AS archived,
                    deleted_at IS NOT NULL AS deleted, NULL::text AS path, updated_at
             FROM projects WHERE id=$1 AND version=$2"
        }
        SourceType::Task => {
            "SELECT title, CASE WHEN deleted_at IS NULL THEN COALESCE(description, '') ELSE '' END AS body,
                    NULL::jsonb AS document, project_id, archived_at IS NOT NULL AS archived,
                    deleted_at IS NOT NULL AS deleted, NULL::text AS path, updated_at
             FROM tasks WHERE id=$1 AND version=$2"
        }
        SourceType::File => {
            "SELECT regexp_replace(source.relative_path, '^.*/', '') AS title,
                    COALESCE((SELECT string_agg(block->>'text', E'\\n' ORDER BY ord)
                      FROM jsonb_array_elements(extraction.blocks) WITH ORDINALITY blocks(block, ord)
                    ), '') AS body, NULL::jsonb AS document, NULL::uuid AS project_id,
                    false AS archived, false AS deleted, source.relative_path AS path, source.updated_at
             FROM file_sources source JOIN file_extractions extraction
               ON extraction.source_id=source.id AND extraction.content_version=source.content_version
             WHERE source.id=$1 AND source.content_version=$2 AND extraction.id=$3"
        }
        SourceType::ChatMessage => {
            return super::chat_projection::document(tx, event.source_id, event.source_version).await;
        }
    };
    let mut query = sqlx::query_as::<_, ProjectionRow>(sql)
        .bind(event.source_id)
        .bind(event.source_version);
    if source == SourceType::File {
        query = query.bind(event.extraction_id);
    }
    let Some(mut row) = query.fetch_optional(&mut **tx).await? else {
        // The source was removed, or a newer version has its own event.
        return Ok(None);
    };

    if row.deleted {
        // A content-free tombstone drives T53 cleanup for earlier versions.
        // Sources deleted before they were indexed have no keys to retire.
        let had_projection: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM source_documents WHERE source_type=$1 AND source_id=$2)",
        )
        .bind(source.as_str())
        .bind(event.source_id)
        .fetch_one(&mut **tx)
        .await?;
        if !had_projection {
            return Ok(None);
        }
        row.title = "Deleted source".into();
    }

    if let Some(document) = row.document {
        let result = validate_document(
            ctx,
            ValidateDocument {
                format_version: 1,
                document_json: document.to_string(),
            },
        )
        .and_then(|document| extract_plain_text(ctx, ExtractPlainText { document }));
        match result {
            Ok(content) => row.body = content.text,
            Err(_) => {
                tracing::warn!("invalid stored note document in the full-text projection");
                return Ok(None);
            }
        }
    }

    // T53 bounds the document projection. Retain the full text in the source
    // and extracted blocks so downstream chunking can operate independently.
    if row.body.len() > meili::MAX_BODY_BYTES {
        row.body
            .truncate(row.body.floor_char_boundary(meili::MAX_BODY_BYTES));
    }

    let format = row.path.as_ref().and_then(|path| {
        std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| ext.len() <= 32)
            .map(str::to_ascii_lowercase)
    });
    Ok(Some(meili::SourceDocument {
        source_type: source,
        source_id: event.source_id,
        source_version: event.source_version,
        title: row.title,
        body: row.body,
        project_id: row.project_id,
        tags: Vec::new(), // Association-only changes are read from PG.
        archived: row.archived,
        deleted: row.deleted,
        path: row.path,
        format,
        updated_at: row.updated_at,
    }))
}

/// Converts a bounded batch of domain outbox events into T53 publication jobs.
/// A source event and its job are acknowledged together, so retry and multiple
/// workers cannot queue the same event twice.
pub async fn project_pending(ctx: &AppContext) -> Result<usize, meili::DomainError> {
    if !ctx.search().enabled {
        return Ok(0);
    }
    for index in 0..PROJECTION_BATCH {
        let mut tx = ctx.pool().begin().await?;
        let pending: Option<(Uuid, String, serde_json::Value)> = sqlx::query_as(
            "SELECT id, topic, payload FROM outbox WHERE published_at IS NULL
             AND topic IN ('note.changed', 'project.changed', 'task.changed', 'file.extracted', 'chat.changed')
             ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, topic, payload)) = pending else {
            tx.commit().await?;
            return Ok(index);
        };
        let source = match topic.as_str() {
            "note.changed" => SourceType::Note,
            "project.changed" => SourceType::Project,
            "task.changed" => SourceType::Task,
            "file.extracted" => SourceType::File,
            "chat.changed" => SourceType::ChatMessage,
            _ => unreachable!("outbox query restricts topics"),
        };
        match serde_json::from_value(payload) {
            Ok(event) => {
                if let Some(document) = source_document(ctx, &mut tx, source, event).await? {
                    match meili::publish_fulltext_in(ctx, &mut tx, document).await {
                        Ok(_) | Err(meili::DomainError::Stale) => {}
                        Err(meili::DomainError::Invalid(_)) => {
                            tracing::warn!("source exceeds the full-text projection contract");
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(_) => tracing::warn!("invalid source event in the full-text outbox"),
        }
        sqlx::query("UPDATE outbox SET published_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(PROJECTION_BATCH)
}
