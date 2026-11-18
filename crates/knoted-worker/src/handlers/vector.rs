//! Turns current chunk sets into dense vectors through the shared job runner.

use chrono::Duration;
use knoted_core::{
    context::AppContext,
    intelligence::ExecutionRequest,
    jobs::{self, ExecutionOutcome, Job, Lease},
    rag::{
        chunking::{CHUNKER_REVISION, ChunkSet},
        embedding::{self, CHUNKS_PER_EXECUTION, MODEL, MODEL_REVISION},
        vector::{self, publish_claimed_vectors},
    },
    search::chunks,
};
use sqlx::types::Json;
use uuid::Uuid;

pub const KIND: &str = "rag_vectors.embed";
const BATCH_DEADLINE_SECONDS: i64 = 900;

pub async fn schedule(ctx: AppContext) -> ! {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if enqueue_missing(&ctx).await.is_err() {
            eprintln!("knoted-worker: vector scheduling unavailable");
        }
    }
}

async fn enqueue_missing(ctx: &AppContext) -> Result<(), knoted_core::db::DbError> {
    let mut tx = ctx.pool().begin().await?;
    let sets: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM rag_chunk_sets s
         JOIN current_rag_sources live USING (source_type,source_id,source_version,extraction_id)
         WHERE s.chunker_revision=$1
           AND EXISTS (SELECT 1 FROM rag_chunks c WHERE c.chunk_set_id=s.id)
           AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.kind=$2
                     AND j.idempotency_key = s.id::text || ':' || $3)
           AND NOT EXISTS (SELECT 1 FROM rag_embedding_sets e WHERE e.chunk_set_id=s.id
                     AND e.model=$4 AND e.model_revision=$3)
         ORDER BY s.created_at,s.id LIMIT 20",
    )
    .bind(CHUNKER_REVISION)
    .bind(KIND)
    .bind(MODEL_REVISION)
    .bind(MODEL)
    .fetch_all(&mut *tx)
    .await?;
    for set_id in sets {
        let mut spec = jobs::JobSpec::new(KIND);
        spec.idempotency_key = Some(format!("{set_id}:{MODEL_REVISION}"));
        spec.payload = serde_json::json!({"chunk_set_id": set_id});
        jobs::enqueue(&mut tx, spec).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("stale vector source or lease")]
    Stale,
    #[error("invalid vector job envelope")]
    Invalid,
    #[error("earlier embedding exit not confirmed")]
    ExitUnconfirmed,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Chunk(#[from] chunks::DomainError),
    #[error(transparent)]
    Embed(#[from] embedding::DomainError),
    #[error(transparent)]
    Publish(#[from] vector::DomainError),
}

pub async fn execute(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    if !ctx.vectors_enabled() {
        return ExecutionOutcome::Cancelled;
    }
    match run_attempt(ctx, lease, job).await {
        Ok(()) => ExecutionOutcome::Succeeded,
        Err(Failure::Stale | Failure::Publish(vector::DomainError::Stale)) => {
            ExecutionOutcome::Cancelled
        }
        Err(
            Failure::Invalid
            | Failure::Embed(
                embedding::DomainError::InvalidRequest | embedding::DomainError::InvalidArtifact,
            )
            | Failure::Publish(vector::DomainError::Invalid),
        ) => ExecutionOutcome::PermanentFailure {
            message: "embedding data or artifact is invalid".into(),
        },
        Err(_) => ExecutionOutcome::TemporaryFailure {
            message: "embedding execution unavailable".into(),
        },
    }
}

async fn run_attempt(ctx: &AppContext, lease: &Lease, job: &Job) -> Result<(), Failure> {
    let id = job
        .payload
        .get("chunk_set_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or(Failure::Invalid)?;
    // This also settles earlier attempts from a previous Rust worker and the
    // original job referenced by a platform-level manual retry.
    settle_requests(ctx, job.id, job.attempts, job.retry_of).await?;
    let set = chunks::load_chunk_set(ctx, id)
        .await?
        .ok_or(Failure::Stale)?;
    if !chunks::chunk_set_is_current(ctx, &set).await? {
        return Err(Failure::Stale);
    }
    let requests = stage_attempt(ctx, lease, job.attempts, &set).await?;
    let result = embedding::embed_chunks(ctx, &set, &requests).await;
    if result.is_err() {
        settle_requests(ctx, job.id, job.attempts.saturating_add(1), None).await?;
    }
    publish_claimed_vectors(ctx, lease, &result?).await?;
    Ok(())
}

async fn stage_attempt(
    ctx: &AppContext,
    lease: &Lease,
    attempt: i32,
    set: &ChunkSet,
) -> Result<Vec<ExecutionRequest>, Failure> {
    let mut tx = ctx.pool().begin().await?;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE id=$1 AND state='running' AND lease_token=$2
         AND lease_expires_at>clock_timestamp() FOR UPDATE",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(Failure::Stale);
    }
    let existing: Vec<Json<ExecutionRequest>> = sqlx::query_scalar(
        "SELECT request FROM rag_embedding_executions WHERE job_id=$1 AND attempt=$2 ORDER BY ordinal",
    )
    .bind(lease.job_id)
    .bind(attempt)
    .fetch_all(&mut *tx)
    .await?;
    if !existing.is_empty() {
        return Ok(existing.into_iter().map(|value| value.0).collect());
    }
    let batches = set.chunks.len().div_ceil(CHUNKS_PER_EXECUTION);
    let deadline = ctx.now() + Duration::seconds(BATCH_DEADLINE_SECONDS);
    let mut requests = Vec::with_capacity(batches);
    for index in 0..batches {
        let request = embedding::stage_embedding_batch(ctx, set, index, Uuid::new_v4(), deadline)?;
        sqlx::query(
            "INSERT INTO rag_embedding_executions (job_id,attempt,ordinal,request)
             VALUES ($1,$2,$3,$4)",
        )
        .bind(lease.job_id)
        .bind(attempt)
        .bind(i32::try_from(index).map_err(|_| Failure::Invalid)?)
        .bind(Json(&request))
        .execute(&mut *tx)
        .await?;
        requests.push(request);
    }
    tx.commit().await?;
    Ok(requests)
}

async fn settle_requests(
    ctx: &AppContext,
    job_id: Uuid,
    before_attempt: i32,
    retry_of: Option<Uuid>,
) -> Result<(), Failure> {
    let requests: Vec<Json<ExecutionRequest>> = sqlx::query_scalar(
        "SELECT request FROM rag_embedding_executions
         WHERE (job_id=$1 AND attempt<$2) OR job_id=$3 ORDER BY job_id,attempt,ordinal",
    )
    .bind(job_id)
    .bind(before_attempt)
    .bind(retry_of)
    .fetch_all(ctx.pool())
    .await?;
    for Json(request) in requests {
        if embedding::cancel_embedding(ctx, &request).await.is_err() {
            return Err(Failure::ExitUnconfirmed);
        }
    }
    Ok(())
}

pub async fn cancel_and_wait(ctx: &AppContext, lease: &Lease) -> bool {
    let original: Result<Option<Uuid>, _> =
        sqlx::query_scalar("SELECT retry_of FROM jobs WHERE id=$1")
            .bind(lease.job_id)
            .fetch_one(ctx.pool())
            .await;
    match original {
        Ok(original) => settle_requests(ctx, lease.job_id, i32::MAX, original)
            .await
            .is_ok(),
        Err(_) => false,
    }
}
