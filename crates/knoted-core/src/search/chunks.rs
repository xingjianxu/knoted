//! Persisted lexical RAG collection. Meilisearch accepts asynchronous writes;
//! readers can only use a fully confirmed set matching current PostgreSQL data.

use crate::{
    context::AppContext,
    files::text::{ExtractedBlock, ExtractedDocument},
    jobs::{self, ExecutionOutcome, Job, JobSpec, Lease},
    notes::document::{self, ExtractPlainText, ValidateDocument},
    rag::chunking::{
        self, BgeTokenizer, CHUNKER_REVISION, Chunk, ChunkConfig, ChunkLocation, ChunkSet,
        ChunkSource,
    },
    search::meili::{self, EngineTask, MeiliClient, SourceType},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction, types::Json};
use std::sync::OnceLock;
use uuid::Uuid;

pub const BUILD_JOB_KIND: &str = "rag_chunks.build";
pub const PUBLISH_JOB_KIND: &str = "rag_chunks.publish";
const ENGINE_BATCH: i64 = 64;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("chunk source or job is no longer current")]
    Stale,
    #[error("invalid chunk source or job envelope")]
    Invalid,
    #[error("chunk tokenizer service unavailable")]
    Tokenizer,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Queue(#[from] crate::db::DbError),
    #[error(transparent)]
    Engine(#[from] meili::DomainError),
}

pub fn chunk_index_uid(ctx: &AppContext) -> String {
    ctx.search().rag_chunk_index_uid.clone()
}

#[derive(sqlx::FromRow)]
struct SourceRow {
    source_type: String,
    source_id: Uuid,
    source_version: i64,
    extraction_id: Uuid,
}

impl SourceRow {
    fn into_source(self) -> Result<ChunkSource, DomainError> {
        Ok(ChunkSource {
            source_type: SourceType::parse(&self.source_type).ok_or(DomainError::Invalid)?,
            source_id: self.source_id,
            source_version: self.source_version,
            extraction_id: self.extraction_id,
        })
    }
}

const SOURCE_CURRENT: &str = "SELECT EXISTS(SELECT 1 FROM rag_source_snapshots
    WHERE source_type=$1 AND source_id=$2 AND source_version=$3 AND extraction_id=$4
      AND (visible OR ($5 AND source_type='file')))";

pub async fn chunk_set_is_current(ctx: &AppContext, set: &ChunkSet) -> Result<bool, DomainError> {
    if set.chunker_revision != CHUNKER_REVISION {
        return Ok(false);
    }
    Ok(current_source_query(set.source, false)
        .fetch_one(ctx.pool())
        .await?)
}

// Re-observation can temporarily revoke a file's visibility without changing
// its content version. Work already enqueued against the same successful
// extraction can finish. Readers always require `visible`.
fn current_source_query(
    source: ChunkSource,
    allow_hidden_file: bool,
) -> sqlx::query::QueryScalar<'static, Postgres, bool, sqlx::postgres::PgArguments> {
    sqlx::query_scalar(SOURCE_CURRENT)
        .bind(source.source_type.as_str())
        .bind(source.source_id)
        .bind(source.source_version)
        .bind(source.extraction_id)
        .bind(allow_hidden_file)
}

#[derive(sqlx::FromRow)]
struct ChunkRow {
    id: Uuid,
    ordinal: i32,
    text: String,
    token_count: i32,
    locations: Json<Vec<ChunkLocation>>,
}

pub async fn load_chunk_set(ctx: &AppContext, id: Uuid) -> Result<Option<ChunkSet>, DomainError> {
    let Some((source, chunker_revision)) =
        sqlx::query_as::<_, (String, Uuid, i64, Uuid, String)>(
            "SELECT source_type,source_id,source_version,extraction_id,chunker_revision FROM rag_chunk_sets WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(ctx.pool())
        .await?
        .map(|(source_type, source_id, source_version, extraction_id, revision)| {
            (SourceRow { source_type, source_id, source_version, extraction_id }, revision)
        })
    else {
        return Ok(None);
    };

    let chunks = sqlx::query_as::<_, ChunkRow>(
        "SELECT id,ordinal,text,token_count,locations FROM rag_chunks WHERE chunk_set_id=$1 ORDER BY ordinal",
    )
    .bind(id)
    .fetch_all(ctx.pool())
    .await?
    .into_iter()
    .map(|row| Chunk {
        id: row.id,
        ordinal: row.ordinal as usize,
        text: row.text,
        token_count: row.token_count as usize,
        locations: row.locations.0,
    })
    .collect();

    Ok(Some(ChunkSet {
        id,
        source: source.into_source()?,
        chunker_revision,
        chunks,
    }))
}

/// A bounded pass discovers new versions and backfills records saved before T57.
/// Idempotency records failed/canceled builds until an explicit job retry.
/// This projection has its own durable bookkeeping independent of document search.
pub async fn enqueue_missing(ctx: &AppContext) -> Result<usize, DomainError> {
    if !ctx.search().enabled {
        return Ok(0);
    }
    let mut tx = ctx.pool().begin().await?;
    // Only one planner at a time, including when multiple workers start at once.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema() || '/rag-chunk-discovery',0))")
        .execute(&mut *tx).await?;
    let capacity: i64 = sqlx::query_scalar(
        "SELECT greatest(0, 16 - count(*)) FROM jobs WHERE kind IN ($1,$2) AND state IN ('queued','running','retry_wait','cancel_requested')",
    )
    .bind(BUILD_JOB_KIND)
    .bind(PUBLISH_JOB_KIND)
    .fetch_one(&mut *tx).await?;
    if capacity == 0 {
        return Ok(0);
    }
    let index_uid = chunk_index_uid(ctx);
    let candidates = sqlx::query_as::<_, SourceRow>(
        "SELECT source_type,source_id,source_version,extraction_id FROM current_rag_sources s
         WHERE NOT EXISTS (
             SELECT 1 FROM jobs j WHERE j.kind=$1 AND j.idempotency_key=concat_ws('/', $2::text,
                 $3::text, s.source_type, s.source_id, s.source_version, s.extraction_id)
         )
         ORDER BY source_type, source_id LIMIT $4",
    )
    .bind(BUILD_JOB_KIND)
    .bind(&index_uid)
    .bind(CHUNKER_REVISION)
    .bind(capacity)
    .fetch_all(&mut *tx)
    .await?;
    let mut created = 0;
    for candidate in candidates {
        let source = candidate.into_source()?;
        let key = format!(
            "{index_uid}/{CHUNKER_REVISION}/{}/{}/{}/{}",
            source.source_type.as_str(),
            source.source_id,
            source.source_version,
            source.extraction_id
        );
        let result = jobs::enqueue(
            &mut tx,
            JobSpec {
                idempotency_key: Some(key),
                priority: -10,
                payload: serde_json::to_value(source).map_err(|_| DomainError::Invalid)?,
                ..JobSpec::new(BUILD_JOB_KIND)
            },
        )
        .await?;
        created += usize::from(result.created);
    }
    tx.commit().await?;
    Ok(created)
}

fn make_block(ordinal: usize, kind: &str, text: String, structure: Value) -> ExtractedBlock {
    ExtractedBlock {
        ordinal,
        kind: kind.into(),
        text,
        page: None,
        bbox: None,
        structure,
    }
}

async fn source_document(
    ctx: &AppContext,
    source: ChunkSource,
) -> Result<ExtractedDocument, DomainError> {
    if !current_source_query(source, true)
        .fetch_one(ctx.pool())
        .await?
    {
        return Err(DomainError::Stale);
    }
    let blocks = match source.source_type {
        SourceType::File => {
            sqlx::query_scalar::<_, Json<Vec<ExtractedBlock>>>(
                "SELECT blocks FROM file_extractions WHERE id=$1",
            )
            .bind(source.extraction_id)
            .fetch_optional(ctx.pool())
            .await?
            .ok_or(DomainError::Stale)?
            .0
        }
        SourceType::Note => {
            let (raw, format): (String, i32) = sqlx::query_as(
                "SELECT document::text,format_version FROM notes WHERE id=$1 AND content_version=$2 AND deleted_at IS NULL",
            ).bind(source.source_id).bind(source.source_version).fetch_optional(ctx.pool()).await?
             .ok_or(DomainError::Stale)?;
            let note = document::validate_document(
                ctx,
                ValidateDocument {
                    document_json: raw,
                    format_version: format as u32,
                },
            )
            .map_err(|_| DomainError::Invalid)?;
            let decoded: Value =
                serde_json::from_str(note.document_json()).map_err(|_| DomainError::Invalid)?;
            let plain = document::extract_plain_text(ctx, ExtractPlainText { document: note })
                .map_err(|_| DomainError::Invalid)?;
            plain
                .blocks
                .into_iter()
                .enumerate()
                .map(|(ordinal, block)| {
                    let mut node = &decoded;
                    let mut key = "blocks";
                    for step in &block.path {
                        node = &node[key][*step];
                        key = "children";
                    }
                    make_block(
                        ordinal,
                        node["type"].as_str().unwrap_or("paragraph"),
                        block.text,
                        json!({"block_id":block.block_id,"path":block.path}),
                    )
                })
                .collect()
        }
        SourceType::Task => {
            let (title, description): (String, Option<String>) = sqlx::query_as(
                "SELECT title,description FROM tasks WHERE id=$1 AND version=$2 AND deleted_at IS NULL",
            ).bind(source.source_id).bind(source.source_version).fetch_optional(ctx.pool()).await?
             .ok_or(DomainError::Stale)?;
            let mut blocks = vec![make_block(0, "title", title, json!({"field":"title"}))];
            if let Some(description) = description {
                blocks.push(make_block(
                    1,
                    "paragraph",
                    description,
                    json!({"field":"description"}),
                ));
            }
            let activities: Vec<(Uuid, i64, String)> = sqlx::query_as(
                "SELECT id,version,body FROM activities WHERE task_id=$1 AND deleted_at IS NULL
                 AND author_kind='manual' ORDER BY created_at,id",
            )
            .bind(source.source_id)
            .fetch_all(ctx.pool())
            .await?;
            for (id, version, body) in activities {
                blocks.push(make_block(
                    blocks.len(),
                    "activity",
                    body,
                    json!({
                        "activity_id":id, "activity_version":version,
                    }),
                ));
            }
            blocks
        }
        _ => return Err(DomainError::Invalid),
    };
    Ok(ExtractedDocument {
        blocks,
        source_format: source.source_type.as_str().into(),
        title: None,
        page_count: None,
        truncated: false,
    })
}

async fn lock_running_lease(
    tx: &mut Transaction<'_, Postgres>,
    lease: &Lease,
) -> Result<(), DomainError> {
    let valid = sqlx::query_scalar::<_, bool>(
        "SELECT lease_token=$2 AND state='running' AND lease_expires_at>clock_timestamp()
         FROM jobs WHERE id=$1 FOR UPDATE",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .fetch_optional(&mut **tx)
    .await?;
    if valid != Some(true) {
        return Err(DomainError::Stale);
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
struct Publish {
    chunk_set_id: Uuid,
    index_uid: String,
}

async fn persist(ctx: &AppContext, lease: &Lease, set: ChunkSet) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lock_running_lease(&mut tx, lease).await?;
    if !current_source_query(set.source, true)
        .fetch_one(&mut *tx)
        .await?
    {
        return Err(DomainError::Stale);
    }
    let insert = sqlx::query(
        "INSERT INTO rag_chunk_sets(id,source_type,source_id,source_version,extraction_id,chunker_revision)
         VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING",
    ).bind(set.id).bind(set.source.source_type.as_str()).bind(set.source.source_id)
     .bind(set.source.source_version).bind(set.source.extraction_id).bind(&set.chunker_revision)
     .execute(&mut *tx).await?;
    if insert.rows_affected() == 1 {
        // Pass arrays instead of one round trip per chunk for large sources.
        for batch in set.chunks.chunks(512) {
            let ids = batch.iter().map(|chunk| chunk.id).collect::<Vec<_>>();
            let ordinals = batch
                .iter()
                .map(|chunk| chunk.ordinal as i32)
                .collect::<Vec<_>>();
            let texts = batch
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<Vec<_>>();
            let counts = batch
                .iter()
                .map(|chunk| chunk.token_count as i32)
                .collect::<Vec<_>>();
            let locations = batch
                .iter()
                .map(|chunk| Json(&chunk.locations))
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO rag_chunks(id,chunk_set_id,ordinal,text,token_count,locations)
                 SELECT id,$1,ordinal,text,token_count,locations
                 FROM unnest($2::uuid[],$3::integer[],$4::text[],$5::integer[],$6::jsonb[])
                      AS row(id,ordinal,text,token_count,locations)",
            )
            .bind(set.id)
            .bind(ids)
            .bind(ordinals)
            .bind(texts)
            .bind(counts)
            .bind(locations)
            .execute(&mut *tx)
            .await?;
        }
    }
    let input = Publish {
        chunk_set_id: set.id,
        index_uid: chunk_index_uid(ctx),
    };
    let enqueued = jobs::enqueue(
        &mut tx,
        JobSpec {
            idempotency_key: Some(format!("{}/{}", input.index_uid, input.chunk_set_id)),
            priority: -10,
            payload: serde_json::to_value(&input).map_err(|_| DomainError::Invalid)?,
            ..JobSpec::new(PUBLISH_JOB_KIND)
        },
    )
    .await?;
    sqlx::query(
        "INSERT INTO rag_chunk_publications(chunk_set_id,index_uid,job_id) VALUES($1,$2,$3)
         ON CONFLICT (chunk_set_id,index_uid) DO NOTHING",
    )
    .bind(set.id)
    .bind(input.index_uid)
    .bind(enqueued.job.id)
    .execute(&mut *tx)
    .await?;
    lock_running_lease(&mut tx, lease).await?;
    if !current_source_query(set.source, true)
        .fetch_one(&mut *tx)
        .await?
    {
        return Err(DomainError::Stale);
    }
    tx.commit().await?;
    Ok(())
}

async fn build(ctx: &AppContext, lease: &Lease, job: &Job) -> Result<(), DomainError> {
    let source: ChunkSource =
        serde_json::from_value(job.payload.clone()).map_err(|_| DomainError::Invalid)?;
    let document = source_document(ctx, source).await?;
    let set = tokio::task::spawn_blocking(move || {
        static TOKENIZER: OnceLock<BgeTokenizer> = OnceLock::new();
        if TOKENIZER.get().is_none() {
            let tokenizer = BgeTokenizer::from_env().map_err(|_| DomainError::Tokenizer)?;
            let _ = TOKENIZER.set(tokenizer);
        }
        let tokenizer = TOKENIZER.get().ok_or(DomainError::Tokenizer)?;
        chunking::chunk_document(&document, source, &ChunkConfig::default(), tokenizer).map_err(
            |error| match error {
                chunking::DomainError::InvalidSource => DomainError::Invalid,
                _ => DomainError::Tokenizer,
            },
        )
    })
    .await
    .map_err(|_| DomainError::Tokenizer)??;
    persist(ctx, lease, set).await
}

#[derive(sqlx::FromRow)]
struct Publication {
    job_id: Uuid,
    status: String,
    meili_task_uid: Option<i64>,
    next_ordinal: i32,
    source_type: String,
    source_id: Uuid,
    source_version: i64,
    extraction_id: Uuid,
    chunker_revision: String,
}

impl Publication {
    fn source(&self) -> Result<ChunkSource, DomainError> {
        SourceRow {
            source_type: self.source_type.clone(),
            source_id: self.source_id,
            source_version: self.source_version,
            extraction_id: self.extraction_id,
        }
        .into_source()
    }
}

async fn publication(ctx: &AppContext, input: &Publish) -> Result<Publication, DomainError> {
    sqlx::query_as(
        "SELECT p.job_id,p.status,p.meili_task_uid,p.next_ordinal,s.source_type,s.source_id,
                s.source_version,s.extraction_id,s.chunker_revision
         FROM rag_chunk_publications p JOIN rag_chunk_sets s ON s.id=p.chunk_set_id
         WHERE p.chunk_set_id=$1 AND p.index_uid=$2",
    )
    .bind(input.chunk_set_id)
    .bind(&input.index_uid)
    .fetch_optional(ctx.pool())
    .await?
    .ok_or(DomainError::Stale)
}

async fn adopt_retry(
    ctx: &AppContext,
    lease: &Lease,
    job: &Job,
    input: &Publish,
    record: &Publication,
) -> Result<(), DomainError> {
    if record.job_id == lease.job_id {
        return Ok(());
    }
    if job.retry_of != Some(record.job_id) {
        return Err(DomainError::Stale);
    }
    let mut tx = ctx.pool().begin().await?;
    lock_running_lease(&mut tx, lease).await?;
    let adopted = sqlx::query(
        "UPDATE rag_chunk_publications p SET job_id=$3, status='pending', meili_task_uid=NULL, updated_at=now()
         FROM jobs original WHERE p.chunk_set_id=$1 AND p.index_uid=$2
             AND original.id=$4 AND p.job_id=$4 AND original.state IN ('failed','cancelled')
             AND p.status <> 'published'",
    ).bind(input.chunk_set_id).bind(&input.index_uid).bind(lease.job_id).bind(record.job_id)
     .execute(&mut *tx).await?;
    if adopted.rows_affected() != 1 {
        return Err(DomainError::Stale);
    }
    tx.commit().await?;
    Ok(())
}

async fn progress(
    ctx: &AppContext,
    lease: &Lease,
    input: &Publish,
    source: ChunkSource,
    status: &str,
    next_ordinal: i32,
    task: Option<i64>,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lock_running_lease(&mut tx, lease).await?;
    if status != "failed"
        && !current_source_query(source, true)
            .fetch_one(&mut *tx)
            .await?
    {
        return Err(DomainError::Stale);
    }
    let written = sqlx::query(
        "UPDATE rag_chunk_publications SET status=$4, next_ordinal=$5, meili_task_uid=$6,
             last_confirmed_meili_task_uid=CASE WHEN $4='pending' AND status='submitted'
                 THEN meili_task_uid ELSE last_confirmed_meili_task_uid END,
             published_at=CASE WHEN $4='published' THEN now() ELSE NULL END, updated_at=now()
         WHERE chunk_set_id=$1 AND index_uid=$2 AND job_id=$3 AND status <> 'published'",
    )
    .bind(input.chunk_set_id)
    .bind(&input.index_uid)
    .bind(lease.job_id)
    .bind(status)
    .bind(next_ordinal)
    .bind(task)
    .execute(&mut *tx)
    .await?;
    if written.rows_affected() != 1 {
        return Err(DomainError::Stale);
    }
    tx.commit().await?;
    Ok(())
}

async fn publish(
    ctx: &AppContext,
    lease: &Lease,
    input: &Publish,
    record: Publication,
) -> Result<(), DomainError> {
    let source = record.source()?;
    if record.chunker_revision != CHUNKER_REVISION
        || !current_source_query(source, true)
            .fetch_one(ctx.pool())
            .await?
    {
        return Err(DomainError::Stale);
    }
    if record.status == "published" {
        return Ok(());
    }
    let engine = MeiliClient::from_context(ctx)?;
    engine.ensure_index_settings(&input.index_uid, &json!({
        "searchableAttributes": ["text"],
        "filterableAttributes": ["chunk_id", "source_type", "source_id", "source_version", "chunk_set_id", "project_id"],
        "sortableAttributes": [],
    })).await?;

    let mut ordinal = record.next_ordinal;
    let mut task = record.meili_task_uid;
    loop {
        let chunks: Vec<(Uuid, String, Value)> = sqlx::query_as(
            "SELECT id,text,locations FROM rag_chunks WHERE chunk_set_id=$1 AND ordinal >= $2 ORDER BY ordinal LIMIT $3",
        ).bind(input.chunk_set_id).bind(ordinal).bind(ENGINE_BATCH).fetch_all(ctx.pool()).await?;
        if chunks.is_empty() {
            return progress(ctx, lease, input, source, "published", ordinal, None).await;
        }
        let task_uid = match task {
            Some(uid) => uid,
            None => {
                let project: Option<(Option<Uuid>,)> = sqlx::query_as(
                    "SELECT project_id FROM rag_source_snapshots WHERE source_type=$1 AND source_id=$2
                        AND source_version=$3 AND extraction_id=$4 AND (visible OR source_type='file')",
                ).bind(source.source_type.as_str()).bind(source.source_id).bind(source.source_version)
                 .bind(source.extraction_id).fetch_optional(ctx.pool()).await?;
                let (project_id,) = project.ok_or(DomainError::Stale)?;
                let docs = chunks
                    .iter()
                    .map(|(id, text, locations)| {
                        json!({
                            "id":id, "chunk_id":id, "chunk_set_id":input.chunk_set_id,
                            "source_type":source.source_type, "source_id":source.source_id,
                            "source_version":source.source_version, "project_id":project_id,
                            "text":text, "location":locations,
                        })
                    })
                    .collect::<Vec<_>>();
                let uid = engine.submit_documents(&input.index_uid, &docs).await?;
                progress(ctx, lease, input, source, "submitted", ordinal, Some(uid)).await?;
                uid
            }
        };
        match engine.wait_task(task_uid).await? {
            EngineTask::Succeeded => {
                ordinal += chunks.len() as i32;
                progress(ctx, lease, input, source, "pending", ordinal, None).await?;
                task = None;
            }
            EngineTask::Failed { .. } | EngineTask::Canceled => {
                return Err(
                    meili::DomainError::Rejected("chunk index rejected the batch".into()).into(),
                );
            }
            EngineTask::Enqueued | EngineTask::Processing => {
                return Err(meili::DomainError::Unavailable.into());
            }
        }
    }
}

pub async fn execute(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    if job.kind == BUILD_JOB_KIND {
        return outcome(build(ctx, lease, job).await);
    }
    if job.kind != PUBLISH_JOB_KIND {
        return outcome(Err(DomainError::Invalid));
    }
    let Ok(input) = serde_json::from_value::<Publish>(job.payload.clone()) else {
        return outcome(Err(DomainError::Invalid));
    };
    let result = async {
        let mut record = publication(ctx, &input).await?;
        adopt_retry(ctx, lease, job, &input, &record).await?;
        if record.job_id != lease.job_id {
            record = publication(ctx, &input).await?;
        }
        let source = record.source()?;
        let result = outcome(publish(ctx, lease, &input, record).await);
        let exhausted = matches!(result, ExecutionOutcome::TemporaryFailure { .. })
            && job.attempts >= job.max_attempts;
        if matches!(result, ExecutionOutcome::PermanentFailure { .. }) || exhausted {
            let current = publication(ctx, &input).await?;
            progress(
                ctx,
                lease,
                &input,
                source,
                "failed",
                current.next_ordinal,
                None,
            )
            .await?;
        }
        Ok(result)
    }
    .await;
    match result {
        Ok(result) => result,
        Err(error) => outcome(Err(error)),
    }
}

fn outcome(result: Result<(), DomainError>) -> ExecutionOutcome {
    match result {
        Ok(()) => ExecutionOutcome::Succeeded,
        Err(DomainError::Stale) => ExecutionOutcome::Cancelled,
        Err(DomainError::Invalid | DomainError::Engine(meili::DomainError::Rejected(_))) => {
            ExecutionOutcome::PermanentFailure {
                message: "chunk projection or engine request rejected".into(),
            }
        }
        Err(DomainError::Tokenizer) => ExecutionOutcome::TemporaryFailure {
            message: "BGE-M3 tokenizer unavailable".into(),
        },
        Err(_) => ExecutionOutcome::TemporaryFailure {
            message: "chunk publication unavailable".into(),
        },
    }
}
