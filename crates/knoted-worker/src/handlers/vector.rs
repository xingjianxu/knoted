//! Turns current chunk sets into dense vectors through the shared job runner.

use chrono::{DateTime, Duration, Utc};
use knoted_core::{
    context::AppContext,
    intelligence::ExecutionRequest,
    jobs::{self, ExecutionOutcome, Job, Lease},
    rag::{
        chunking::{CHUNKER_REVISION, ChunkSet},
        embedding::{self, CHUNKS_PER_EXECUTION, EmbeddingSet, MODEL, MODEL_REVISION},
        vector::{self, publish_claimed_vectors},
    },
    search::chunks,
};
use sqlx::types::Json;
use uuid::Uuid;

pub const KIND: &str = "rag_vectors.embed";
const BATCH_DEADLINE_SECONDS: i64 = 900;

pub async fn schedule(ctx: AppContext, mut stop: tokio::sync::watch::Receiver<()>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while super::next_tick(&mut tick, &mut stop).await {
        let Ok(Some(admission)) = ctx.try_write().await else {
            continue;
        };
        if enqueue_missing(&ctx).await.is_err() {
            eprintln!("knoted-worker: vector scheduling unavailable");
        }
        if admission.finish().await.is_err() {
            eprintln!("knoted-worker: maintenance completion could not be recorded");
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
    // Never begin a new computation until earlier possible dispatches for
    // this job and its manual retry chain have a proof of exit.
    settle_requests(ctx, lease, false).await?;
    let set = chunks::load_chunk_set(ctx, id)
        .await?
        .ok_or(Failure::Stale)?;
    if !chunks::chunk_set_is_current(ctx, &set).await? {
        return Err(Failure::Stale);
    }
    let deadline = ctx.now() + Duration::seconds(BATCH_DEADLINE_SECONDS);
    let mut rows = Vec::with_capacity(set.chunks.len());
    for index in 0..set.chunks.len().div_ceil(CHUNKS_PER_EXECUTION) {
        let request = stage_batch(ctx, lease, &set, index, deadline).await?;
        let result = embedding::embed_chunk_batch(ctx, &set, index, &request).await;
        match result {
            Ok(batch) => rows.extend(batch),
            Err(error) => {
                // Prior batches already proved exit. Only this batch may have
                // reached Python without returning that proof.
                embedding::cancel_embedding(ctx, &request)
                    .await
                    .map_err(|_| Failure::ExitUnconfirmed)?;
                return Err(Failure::Embed(error));
            }
        }
    }
    publish_claimed_vectors(
        ctx,
        lease,
        &EmbeddingSet {
            chunk_set_id: set.id,
            model: MODEL.into(),
            model_revision: MODEL_REVISION.into(),
            rows,
        },
    )
    .await?;
    Ok(())
}

async fn stage_batch(
    ctx: &AppContext,
    lease: &Lease,
    set: &ChunkSet,
    index: usize,
    deadline: DateTime<Utc>,
) -> Result<ExecutionRequest, Failure> {
    let ordinal = i32::try_from(index).map_err(|_| Failure::Invalid)?;
    let mut tx = ctx.pool().begin().await?;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE id=$1 AND state='running' AND lease_token=$2
         AND attempts=$3 AND lease_expires_at>clock_timestamp() FOR UPDATE",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(lease.attempt)
    .fetch_optional(&mut *tx)
    .await?;
    if owned.is_none() {
        return Err(Failure::Stale);
    }
    let existing: Option<Json<ExecutionRequest>> = sqlx::query_scalar(
        "SELECT request FROM rag_embedding_executions
         WHERE job_id=$1 AND attempt=$2 AND ordinal=$3",
    )
    .bind(lease.job_id)
    .bind(lease.attempt)
    .bind(ordinal)
    .fetch_optional(&mut *tx)
    .await?;
    let request = match existing {
        Some(Json(request)) => request,
        None => {
            let request =
                embedding::stage_embedding_batch(ctx, set, index, Uuid::new_v4(), deadline)?;
            sqlx::query(
                "INSERT INTO rag_embedding_executions (job_id,attempt,ordinal,request)
                 VALUES ($1,$2,$3,$4)",
            )
            .bind(lease.job_id)
            .bind(lease.attempt)
            .bind(ordinal)
            .bind(Json(&request))
            .execute(&mut *tx)
            .await?;
            request
        }
    };
    // Only this batch may be replayed if the server's reply is lost. Later
    // batches have no durable receipt until just before their own dispatch.
    tx.commit().await?;
    Ok(request)
}

async fn settle_requests(
    ctx: &AppContext,
    lease: &Lease,
    include_current: bool,
) -> Result<(), Failure> {
    // Check ownership and fetch receipts in one snapshot. A revoked lease may
    // settle only its own attempt. Taking over a cancellation can reuse its
    // attempt number, but it cannot dispatch a new generation in that state.
    let requests: Vec<Json<ExecutionRequest>> = sqlx::query_scalar(
        "WITH RECURSIVE retry_chain AS (
           SELECT id,retry_of FROM jobs WHERE id=$1 AND lease_token=$2
           UNION
           SELECT prior.id,prior.retry_of FROM jobs prior JOIN retry_chain next ON prior.id=next.retry_of
         )
         SELECT request FROM rag_embedding_executions
         WHERE (job_id=$1 AND attempt=$3 AND $4)
            OR (job_id IN (SELECT id FROM retry_chain) AND (job_id<>$1 OR attempt<$3 OR $4))
         ORDER BY job_id,attempt,ordinal",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(lease.attempt)
    .bind(include_current)
    .fetch_all(ctx.pool())
    .await?;
    for Json(request) in requests {
        embedding::cancel_embedding(ctx, &request)
            .await
            .map_err(|_| Failure::ExitUnconfirmed)?;
    }
    Ok(())
}

pub async fn cancel_and_wait(ctx: &AppContext, lease: &Lease) -> bool {
    settle_requests(ctx, lease, true).await.is_ok()
}
