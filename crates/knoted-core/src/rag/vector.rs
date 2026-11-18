//! Atomic publication and source-version-scoped cosine search.

use sqlx::{PgConnection, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::{context::AppContext, search::meili::SourceType};

use super::{
    chunking::CHUNKER_REVISION,
    embedding::{DIMENSIONS, EmbeddingSet},
};

const MAX_CANDIDATES: u32 = 40;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid vector input or model identity")]
    Invalid,
    #[error("vector source is no longer current")]
    Stale,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug)]
pub struct SearchVectors {
    pub model: String,
    pub model_revision: String,
    pub query: Vec<f32>,
    pub source_types: Vec<SourceType>,
    pub project_ids: Vec<Uuid>,
    pub tag_ids: Vec<Uuid>,
    pub limit: u32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct VectorCandidate {
    pub embedding_set_id: Uuid,
    pub chunk_id: Uuid,
    pub chunk_set_id: Uuid,
    pub source_id: Uuid,
    pub source_type: String,
    pub source_version: i64,
    pub distance: f64,
}

fn model_id_valid(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128
}

fn vector_literal(vector: &[f32]) -> Result<String, DomainError> {
    let norm_squared = vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>();
    if vector.len() != DIMENSIONS
        || !norm_squared.is_finite()
        || (norm_squared.sqrt() - 1.0).abs() > 1e-4
    {
        return Err(DomainError::Invalid);
    }
    serde_json::to_string(vector).map_err(|_| DomainError::Invalid)
}

pub async fn publish_vectors(ctx: &AppContext, set: &EmbeddingSet) -> Result<Uuid, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let id = publish_in(&mut tx, set).await?;
    tx.commit().await?;
    Ok(id)
}

pub(crate) async fn publish_in(
    db: &mut PgConnection,
    set: &EmbeddingSet,
) -> Result<Uuid, DomainError> {
    if !model_id_valid(&set.model) || !model_id_valid(&set.model_revision) || set.rows.is_empty() {
        return Err(DomainError::Invalid);
    }
    let mut ids = Vec::with_capacity(set.rows.len());
    let mut literals = Vec::with_capacity(set.rows.len());
    for row in &set.rows {
        ids.push(row.chunk_id);
        literals.push(vector_literal(&row.vector)?);
    }

    // Lock the immutable set for idempotent replays. Publication additionally
    // requires the same visible source revision and extraction.
    let current: Option<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM rag_chunk_sets s JOIN current_rag_sources live
           USING (source_type,source_id,source_version,extraction_id)
         WHERE s.id=$1 AND s.chunker_revision=$2 FOR UPDATE OF s",
    )
    .bind(set.chunk_set_id)
    .bind(CHUNKER_REVISION)
    .fetch_optional(&mut *db)
    .await?;
    if current.is_none() {
        return Err(DomainError::Stale);
    }
    let wanted: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM rag_chunks WHERE chunk_set_id=$1 ORDER BY ordinal")
            .bind(set.chunk_set_id)
            .fetch_all(&mut *db)
            .await?;
    if ids != wanted {
        return Err(DomainError::Invalid);
    }

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO rag_embedding_sets (chunk_set_id,model,model_revision)
         VALUES ($1,$2,$3)
         ON CONFLICT (chunk_set_id,model,model_revision) DO NOTHING RETURNING id",
    )
    .bind(set.chunk_set_id)
    .bind(&set.model)
    .bind(&set.model_revision)
    .fetch_optional(&mut *db)
    .await?;
    if let Some(id) = inserted {
        sqlx::query(
            "INSERT INTO rag_vectors (embedding_set_id,chunk_id,embedding)
             SELECT $1, chunk_id, value::vector FROM unnest($2::uuid[],$3::text[]) AS rows(chunk_id,value)",
        )
        .bind(id)
        .bind(ids)
        .bind(literals)
        .execute(&mut *db)
        .await?;
        return Ok(id);
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM rag_embedding_sets WHERE chunk_set_id=$1 AND model=$2 AND model_revision=$3",
    )
    .bind(set.chunk_set_id)
    .bind(&set.model)
    .bind(&set.model_revision)
    .fetch_one(db)
    .await?)
}

pub async fn search_vectors(
    ctx: &AppContext,
    input: SearchVectors,
) -> Result<Vec<VectorCandidate>, DomainError> {
    if !model_id_valid(&input.model)
        || !model_id_valid(&input.model_revision)
        || !(1..=MAX_CANDIDATES).contains(&input.limit)
        || input
            .source_types
            .iter()
            .any(|kind| !matches!(kind, SourceType::Note | SourceType::File | SourceType::Task))
    {
        return Err(DomainError::Invalid);
    }
    let source_types: Vec<_> = input
        .source_types
        .iter()
        .map(|item| item.as_str())
        .collect();
    let query_vector = vector_literal(&input.query)?;
    let mut query: QueryBuilder<'_, Postgres> = QueryBuilder::new(
        "SELECT candidates.* FROM (
         SELECT e.id AS embedding_set_id,v.chunk_id,s.id AS chunk_set_id,
                s.source_id,s.source_type,s.source_version,v.embedding <=> ",
    );
    query
        .push_bind(query_vector)
        .push(
            "::vector AS distance
         FROM rag_vectors v
         JOIN rag_embedding_sets e ON e.id=v.embedding_set_id
         JOIN rag_chunk_sets s ON s.id=e.chunk_set_id
         JOIN current_rag_sources live USING (source_type,source_id,source_version,extraction_id)
         WHERE e.model=",
        )
        .push_bind(input.model)
        .push(" AND e.model_revision=")
        .push_bind(input.model_revision)
        .push(" AND s.chunker_revision=")
        .push_bind(CHUNKER_REVISION)
        .push(" AND s.source_type=ANY(")
        .push_bind(source_types)
        .push(")");
    push_scope(&mut query, input.project_ids, input.tag_ids);
    query
        .push(" ORDER BY distance LIMIT ")
        .push_bind(i64::from(input.limit))
        .push(") AS candidates ORDER BY distance,chunk_id");
    let mut tx = ctx.pool().begin().await?;
    sqlx::query("SET LOCAL hnsw.iterative_scan = 'strict_order'")
        .execute(&mut *tx)
        .await?;
    let rows = query.build_query_as().fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows)
}

/// Publish just before finishing a claimed job while cancellation remains fenced.
pub async fn publish_claimed_vectors(
    ctx: &AppContext,
    lease: &crate::jobs::Lease,
    set: &EmbeddingSet,
) -> Result<Uuid, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE id=$1 AND lease_token=$2 AND state='running'
         AND lease_expires_at>clock_timestamp() FOR UPDATE",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(DomainError::Stale);
    }
    let id = publish_in(&mut tx, set).await?;
    tx.commit().await?;
    Ok(id)
}

/// The lexical and vector candidates share the same current-source scope.
pub(crate) fn push_scope(
    query: &mut QueryBuilder<'_, Postgres>,
    project_ids: Vec<Uuid>,
    tag_ids: Vec<Uuid>,
) {
    if !project_ids.is_empty() {
        query
            .push(" AND EXISTS (SELECT 1 FROM projects p WHERE p.id=ANY(")
            .push_bind(project_ids)
            .push(
                ") AND p.deleted_at IS NULL AND (p.id=live.project_id OR EXISTS (
                SELECT 1 FROM entity_links l
                WHERE (l.source_id=live.source_id AND l.target_id=p.id)
                   OR (l.source_id=p.id AND l.target_id=live.source_id))))",
            );
    }
    if !tag_ids.is_empty() {
        query.push(" AND NOT EXISTS (SELECT 1 FROM unnest(")
            .push_bind(tag_ids)
            .push("::uuid[]) requested(id)
                WHERE NOT EXISTS (
                    SELECT 1 FROM tags t WHERE t.id=requested.id AND t.deleted_at IS NULL AND (
                          (live.source_type='note' AND EXISTS (
                             SELECT 1 FROM note_tags j WHERE j.entity_id=live.source_id AND j.tag_id=t.id))
                       OR (live.source_type='file' AND EXISTS (
                             SELECT 1 FROM file_tags j WHERE j.entity_id=live.source_id AND j.tag_id=t.id))
                       OR (live.source_type='task' AND EXISTS (
                             SELECT 1 FROM task_tags j WHERE j.entity_id=live.source_id AND j.tag_id=t.id))
                    )
                ))");
    }
}
