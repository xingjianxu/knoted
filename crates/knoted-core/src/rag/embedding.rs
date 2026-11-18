//! Locked, staged embedding of immutable chunks via the Python executor.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    context::AppContext,
    files::{ObjectId, Sha256Hex, StagingError, StagingStore, artifact},
    intelligence::{
        ExecutionKind, ExecutionRequest, ExecutionState, IntelligenceClient, IntelligenceError,
    },
};

use super::chunking::{Chunk, ChunkSet};

pub const MODEL: &str = "BAAI/bge-m3";
pub const MODEL_REVISION: &str = "5617a9f61b028005a4858fdac845db406aefb181";
pub const OPTIONS_REVISION: &str =
    "embed-chunks-v1+bge-m3-5617a9f61b028005a4858fdac845db406aefb181-dense-1024";
pub const DIMENSIONS: usize = 1024;
pub const CHUNKS_PER_EXECUTION: usize = 64;
const MAX_STAGED_BYTES: u64 = 8 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("embedding staging unavailable")]
    StagingUnavailable,
    #[error("invalid embedding request")]
    InvalidRequest,
    #[error("invalid embedding artifact")]
    InvalidArtifact,
    #[error("embedding execution finished in state {0}")]
    Execution(ExecutionState),
    #[error("embedding execution exceeded its deadline")]
    DeadlineExceeded,
    #[error(transparent)]
    Staging(#[from] StagingError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Artifact(#[from] artifact::Error),
}

#[derive(Debug)]
pub struct EmbeddingSet {
    pub chunk_set_id: Uuid,
    pub model: String,
    pub model_revision: String,
    pub rows: Vec<Embedding>,
}

#[derive(Debug)]
pub struct Embedding {
    pub chunk_id: Uuid,
    pub vector: Vec<f32>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    chunk_set_id: Uuid,
    chunks: Vec<ManifestChunk<'a>>,
}

#[derive(Serialize)]
struct ManifestChunk<'a> {
    id: Uuid,
    text: &'a str,
}

#[derive(Deserialize)]
struct Artifact {
    chunk_set_id: Uuid,
    chunk_ids: Vec<Uuid>,
    model: String,
    revision: String,
    dims: usize,
    normalized: bool,
    dtype: String,
    text_count: usize,
    vectors: Vec<Vec<f32>>,
}

fn manifest_bytes(chunk_set_id: Uuid, chunks: &[Chunk]) -> Result<Vec<u8>, DomainError> {
    serde_json::to_vec(&Manifest {
        chunk_set_id,
        chunks: chunks
            .iter()
            .map(|chunk| ManifestChunk {
                id: chunk.id,
                text: &chunk.text,
            })
            .collect(),
    })
    .map_err(|_| DomainError::InvalidRequest)
}

/// Stages a deterministic request. Persist it before sending the execution;
/// transport retries must use the exact same request and execution ID.
pub fn stage_embedding_batch(
    ctx: &AppContext,
    set: &ChunkSet,
    batch_index: usize,
    execution_id: Uuid,
    deadline: DateTime<Utc>,
) -> Result<ExecutionRequest, DomainError> {
    let chunks = set
        .chunks
        .chunks(CHUNKS_PER_EXECUTION)
        .nth(batch_index)
        .ok_or(DomainError::InvalidRequest)?;
    stage_request(
        ctx,
        set.source.source_version,
        execution_id,
        deadline,
        &manifest_bytes(set.id, chunks)?,
    )
}

pub(crate) fn staged_input_id(execution_id: Uuid, digest: &Sha256Hex) -> String {
    format!("embed-{execution_id}-{digest}.json")
}

fn stage_request(
    ctx: &AppContext,
    source_version: i64,
    execution_id: Uuid,
    deadline: DateTime<Utc>,
    bytes: &[u8],
) -> Result<ExecutionRequest, DomainError> {
    let digest = Sha256Hex::digest(bytes);
    let object_id = ObjectId::parse(staged_input_id(execution_id, &digest))?;
    let root = ctx
        .files()
        .staging_root
        .as_ref()
        .ok_or(DomainError::StagingUnavailable)?;
    let store = StagingStore::new(root.join("input"), MAX_STAGED_BYTES)?;
    store.write(&object_id, bytes)?;

    Ok(ExecutionRequest {
        execution_id,
        kind: ExecutionKind::Embed,
        input_object_id: object_id,
        input_sha256: digest,
        source_version,
        deadline,
        options_revision: OPTIONS_REVISION.into(),
    })
}

/// Replays or creates the persisted executions, validates their complete
/// result, then returns one set with the input order preserved.
pub async fn embed_chunks(
    ctx: &AppContext,
    set: &ChunkSet,
    requests: &[ExecutionRequest],
) -> Result<EmbeddingSet, DomainError> {
    let batches = set.chunks.chunks(CHUNKS_PER_EXECUTION);
    if requests.len() != batches.len() {
        return Err(DomainError::InvalidRequest);
    }
    for (chunks, request) in batches.clone().zip(requests) {
        verify_request(set, chunks, request)?;
    }

    let client = IntelligenceClient::new(&ctx.intelligence().base_url)?;
    let mut rows = Vec::with_capacity(set.chunks.len());
    for (chunks, request) in batches.zip(requests) {
        let body = complete_batch(ctx, &client, request).await?;
        rows.extend(decode_artifact(
            set.id,
            chunks.iter().map(|chunk| chunk.id),
            &body,
        )?);
    }
    Ok(EmbeddingSet {
        chunk_set_id: set.id,
        model: MODEL.into(),
        model_revision: MODEL_REVISION.into(),
        rows,
    })
}

/// Execute one batch after persisting its possibly dispatched identity. The
/// caller retains the rows until all batches of the set have been verified.
pub async fn embed_chunk_batch(
    ctx: &AppContext,
    set: &ChunkSet,
    batch_index: usize,
    request: &ExecutionRequest,
) -> Result<Vec<Embedding>, DomainError> {
    let chunks = set
        .chunks
        .chunks(CHUNKS_PER_EXECUTION)
        .nth(batch_index)
        .ok_or(DomainError::InvalidRequest)?;
    verify_request(set, chunks, request)?;
    let client = IntelligenceClient::new(&ctx.intelligence().base_url)?;
    let body = complete_batch(ctx, &client, request).await?;
    decode_artifact(set.id, chunks.iter().map(|chunk| chunk.id), &body)
}

fn verify_request(
    set: &ChunkSet,
    chunks: &[Chunk],
    request: &ExecutionRequest,
) -> Result<(), DomainError> {
    if request.kind != ExecutionKind::Embed
        || request.options_revision != OPTIONS_REVISION
        || request.source_version != set.source.source_version
        || request.input_sha256 != Sha256Hex::digest(&manifest_bytes(set.id, chunks)?)
    {
        return Err(DomainError::InvalidRequest);
    }
    Ok(())
}

/// Questions use the same locked dense space as stored chunks. The ephemeral
/// manifest carries only local execution identities, never a domain source.
pub async fn embed_query(ctx: &AppContext, text: &str) -> Result<Vec<f32>, DomainError> {
    if text.trim().is_empty() {
        return Err(DomainError::InvalidRequest);
    }
    let execution_id = Uuid::new_v4();
    let chunk_id = Uuid::new_v4();
    let bytes = serde_json::to_vec(&Manifest {
        chunk_set_id: execution_id,
        chunks: vec![ManifestChunk { id: chunk_id, text }],
    })
    .map_err(|_| DomainError::InvalidRequest)?;
    let request = stage_request(
        ctx,
        1,
        execution_id,
        ctx.now() + chrono::Duration::seconds(30),
        &bytes,
    )?;
    let client = IntelligenceClient::new(&ctx.intelligence().base_url)?;
    let result = complete_batch(ctx, &client, &request).await;
    if (result.is_ok() || client.settle_execution(&request).await.is_ok())
        && let Some(root) = &ctx.files().staging_root
    {
        let _ =
            tokio::fs::remove_file(root.join("input").join(request.input_object_id.as_str())).await;
    }
    let body = result?;
    decode_artifact(execution_id, [chunk_id].into_iter(), &body)?
        .into_iter()
        .next()
        .map(|row| row.vector)
        .ok_or(DomainError::InvalidArtifact)
}

async fn complete_batch(
    ctx: &AppContext,
    client: &IntelligenceClient,
    request: &ExecutionRequest,
) -> Result<Vec<u8>, DomainError> {
    let mut record = client.create_execution(request).await?.record;
    loop {
        record.verify_echo(request)?;
        if record.state.is_terminal() {
            record.verify_resource_exit(request)?;
            break;
        }
        if ctx.now() >= request.deadline {
            client.settle_execution(request).await?;
            return Err(DomainError::DeadlineExceeded);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        record = client.get_execution(request.execution_id).await?;
    }
    if record.state != ExecutionState::Succeeded {
        return Err(DomainError::Execution(record.state));
    }
    record.verify_fresh(request, ctx.now())?;
    let result = client.get_result(request.execution_id).await?;
    if result.size > MAX_STAGED_BYTES {
        return Err(DomainError::InvalidArtifact);
    }
    Ok(artifact::read_bytes(ctx, request, &result).await?)
}

/// Settle any execution which may have been sent. A lost POST reply is
/// resolved by the same persisted request, and only a proven exit succeeds.
pub async fn cancel_embedding(
    ctx: &AppContext,
    request: &ExecutionRequest,
) -> Result<(), DomainError> {
    let client = IntelligenceClient::new(&ctx.intelligence().base_url)?;
    client.settle_execution(request).await?;
    Ok(())
}

fn decode_artifact(
    set_id: Uuid,
    expected_ids: impl ExactSizeIterator<Item = Uuid>,
    body: &[u8],
) -> Result<Vec<Embedding>, DomainError> {
    let artifact: Artifact =
        serde_json::from_slice(body).map_err(|_| DomainError::InvalidArtifact)?;
    let count = expected_ids.len();
    if artifact.chunk_set_id != set_id
        || artifact.model != MODEL
        || artifact.revision != MODEL_REVISION
        || artifact.dims != DIMENSIONS
        || !artifact.normalized
        || artifact.dtype != "float32"
        || artifact.text_count != count
        || artifact.vectors.len() != count
        || !artifact.chunk_ids.iter().copied().eq(expected_ids)
    {
        return Err(DomainError::InvalidArtifact);
    }
    artifact
        .chunk_ids
        .into_iter()
        .zip(artifact.vectors)
        .map(|(chunk_id, vector)| {
            let norm_squared = vector
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>();
            if vector.len() != DIMENSIONS
                || !norm_squared.is_finite()
                || (norm_squared.sqrt() - 1.0).abs() > 1e-4
            {
                return Err(DomainError::InvalidArtifact);
            }
            Ok(Embedding { chunk_id, vector })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn t58_rejects_malformed_or_mislabeled_output() {
        let set = Uuid::new_v4();
        let chunk = Uuid::new_v4();
        let mut unit = vec![0.0; DIMENSIONS];
        unit[0] = 1.0;
        let artifact = json!({
            "chunk_set_id": set,
            "chunk_ids": [chunk],
            "model": MODEL,
            "revision": MODEL_REVISION,
            "dims": DIMENSIONS,
            "normalized": true,
            "dtype": "float32",
            "text_count": 1,
            "vectors": [unit],
        });
        let verify = |body: &serde_json::Value| {
            decode_artifact(set, [chunk].into_iter(), &serde_json::to_vec(body).unwrap())
        };
        assert_eq!(verify(&artifact).unwrap()[0].chunk_id, chunk);

        for (key, bad_value) in [
            ("vectors", json!([[1.0]])),
            ("vectors", json!([vec![0.0; DIMENSIONS]])),
            ("chunk_ids", json!([Uuid::new_v4()])),
            ("revision", json!("other-model-revision")),
        ] {
            let mut invalid = artifact.clone();
            invalid[key] = bad_value;
            assert!(matches!(
                verify(&invalid),
                Err(DomainError::InvalidArtifact)
            ));
        }
    }
}
