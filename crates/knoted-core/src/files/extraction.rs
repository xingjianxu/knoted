//! Successful extraction publication, fenced independently of metadata CAS.
pub use super::text::{ExtractedBlock, ExtractedDocument};
use super::{ObjectId, Sha256Hex, StagedObject, text};
use crate::{
    context::AppContext,
    jobs::{self, Lease},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct PublicationRoot {
    configured_path: String,
    paused_reason: Option<String>,
    scan_token: Option<Uuid>,
    generation: i64,
    last_complete_generation: Option<i64>,
}

pub const EXTRACT_JOB_KIND: &str = "file.extract";
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("extraction ownership or source observation expired")]
    Stale,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Queue(#[from] crate::db::DbError),
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExtractionInput {
    pub source_id: Uuid,
    pub root_id: Uuid,
    pub content_version: i64,
    pub input_sha256: String,
    pub input_object_id: String,
    pub size: i64,
    pub observation_version: i64,
    pub captured_scan_id: Uuid,
    pub observation: Value,
}
impl ExtractionInput {
    pub fn from_job(job: &jobs::Job) -> Result<Self, serde_json::Error> {
        serde_json::from_value(job.payload.clone())
    }

    pub fn staged_object(&self) -> Result<StagedObject, DomainError> {
        Ok(StagedObject {
            object_id: ObjectId::parse(&self.input_object_id).map_err(|_| DomainError::Stale)?,
            sha256: Sha256Hex::parse(&self.input_sha256).map_err(|_| DomainError::Stale)?,
            size: self.size.try_into().map_err(|_| DomainError::Stale)?,
        })
    }
}
fn options_hash(revision: &str) -> String {
    Sha256Hex::digest(revision.as_bytes()).to_string()
}
/// Caller holds the source lock. Existing successes are immutable and do not
/// depend on a metadata version or the scan generation which later observed it.
pub async fn enqueue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_id: Uuid,
) -> Result<(), DomainError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT relative_path FROM file_sources WHERE id=$1")
            .bind(source_id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((path,)) = row else {
        return Ok(());
    };
    let (parser_revision, options_revision) = if text::supports(&path) {
        (text::PARSER_REVISION, text::OPTIONS_REVISION)
    } else if super::pdf::supports(&path) {
        (super::pdf::PARSER_REVISION, super::pdf::OPTIONS_REVISION)
    } else {
        return Ok(());
    };
    let input:ExtractionInput=sqlx::query_as("SELECT id AS source_id,root_id,content_version,sha256 AS input_sha256,input_object_id,size,observation_version,captured_scan_id,observation FROM file_sources WHERE id=$1").bind(source_id).fetch_one(&mut **tx).await?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM file_extractions WHERE source_id=$1 AND content_version=$2 AND parser_revision=$3 AND options_hash=$4)").bind(source_id).bind(input.content_version).bind(parser_revision).bind(options_hash(options_revision)).fetch_one(&mut **tx).await?;
    if exists {
        return Ok(());
    }
    // Failed semantic work requires a manual retry, including exhausted IO
    // retries. A new capture/scan is not a new parser budget. Legacy T46 jobs
    // omitted these two fields; their fixed v1 identities must not silently
    // change when a future parser/options revision changes the current values.
    let failed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind=$1 AND state='failed' \
         AND payload->>'source_id'=$2 AND payload->>'content_version'=$3 \
         AND COALESCE(payload->>'parser_revision','rust-text-v1')=$4 \
         AND COALESCE(payload->>'options_hash','6541278e7ee4c2460f494c6f29778d64d8fb1f81d5fe19a1ae7e4d914b60272f')=$5)",
    )
    .bind(EXTRACT_JOB_KIND)
    .bind(source_id.to_string())
    .bind(input.content_version.to_string())
    .bind(parser_revision)
    .bind(options_hash(options_revision))
    .fetch_one(&mut **tx)
    .await?;
    if failed {
        return Ok(());
    }
    let mut spec = jobs::JobSpec::new(EXTRACT_JOB_KIND);
    // A newly captured observation can recover from a cancelled stale attempt;
    // replay of the same receipt cannot create an unbounded retry loop.
    spec.idempotency_key = Some(format!(
        "{}:{}:{}:{}:{}:{}",
        source_id,
        input.content_version,
        input.observation_version,
        input.captured_scan_id,
        parser_revision,
        options_hash(options_revision)
    ));
    spec.payload = json!(input);
    spec.payload["parser_revision"] = json!(parser_revision);
    spec.payload["options_hash"] = json!(options_hash(options_revision));
    jobs::enqueue(tx, spec).await?;
    Ok(())
}
async fn lock_lease(conn: &mut sqlx::PgConnection, lease: &Lease) -> Result<(), DomainError> {
    let valid:Option<bool>=sqlx::query_scalar("SELECT state='running' AND lease_token=$2 AND lease_expires_at>clock_timestamp() FROM jobs WHERE id=$1 FOR UPDATE").bind(lease.job_id).bind(lease.token).fetch_optional(conn).await?;
    if valid != Some(true) {
        return Err(DomainError::Stale);
    }
    Ok(())
}
/// Complete publication is atomic with its outbox. Old attempts never overwrite
/// a success, even when replayed after an ambiguous commit response.
pub async fn publish(
    ctx: &AppContext,
    lease: &Lease,
    input: &ExtractionInput,
    document: &ExtractedDocument,
) -> Result<(), DomainError> {
    if !ctx.files().enabled || document.truncated {
        return Err(DomainError::Stale);
    }
    let mut tx = ctx.pool().begin().await?;
    let result = publish_inner(ctx, &mut tx, lease, input, document, None).await;
    match result {
        Ok(()) => {
            tx.commit().await?;
            Ok(())
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}
async fn publish_inner(
    ctx: &AppContext,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    lease: &Lease,
    input: &ExtractionInput,
    document: &ExtractedDocument,
    attempt: Option<&IntelligenceAttempt>,
) -> Result<(), DomainError> {
    lock_lease(tx, lease).await?;
    let (parser_revision, options_revision, execution_id) = if let Some(attempt) = attempt {
        let payload: Value = sqlx::query_scalar("SELECT payload FROM jobs WHERE id=$1")
            .bind(lease.job_id)
            .fetch_one(&mut **tx)
            .await?;
        let entries = decode_attempts(&payload)?;
        if !entries
            .iter()
            .any(|a| a.lease_token == lease.token && a.request == attempt.request)
            || attempt.request.input_sha256.as_str() != input.input_sha256
            || attempt.request.input_object_id.as_str() != input.input_object_id
            || attempt.request.source_version != input.content_version
            || attempt.request.options_revision != super::pdf::OPTIONS_REVISION
        {
            return Err(DomainError::Stale);
        }
        (
            super::pdf::PARSER_REVISION,
            super::pdf::OPTIONS_REVISION,
            Some(attempt.request.execution_id),
        )
    } else {
        (text::PARSER_REVISION, text::OPTIONS_REVISION, None)
    };
    let root:Option<PublicationRoot>=sqlx::query_as("SELECT configured_path,paused_reason,scan_token,generation,last_complete_generation FROM file_roots WHERE id=$1 FOR UPDATE").bind(input.root_id).fetch_optional(&mut **tx).await?;
    let Some(root) = root else {
        return Err(DomainError::Stale);
    };
    if root.paused_reason.is_some()
        || root.scan_token.is_some()
        || Some(root.generation) != root.last_complete_generation
        || ctx.files().root.as_deref() != Some(std::path::Path::new(&root.configured_path))
    {
        return Err(DomainError::Stale);
    }
    let valid:Option<bool>=sqlx::query_scalar("SELECT root_id=$2 AND status='present' AND content_fresh AND content_version=$3 AND sha256=$4 AND input_object_id=$5 AND observation_version=$6 AND captured_observation_version=observation_version AND captured_scan_id=$7 AND observation=$8 AND captured_observation=observation FROM file_sources WHERE id=$1 FOR UPDATE")
 .bind(input.source_id).bind(input.root_id).bind(input.content_version).bind(&input.input_sha256).bind(&input.input_object_id).bind(input.observation_version).bind(input.captured_scan_id).bind(&input.observation).fetch_optional(&mut **tx).await?;
    if valid != Some(true) {
        return Err(DomainError::Stale);
    }
    let inserted:Option<Uuid>=sqlx::query_scalar("INSERT INTO file_extractions(source_id,content_version,input_sha256,parser_revision,options_hash,job_id,blocks,execution_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(source_id,content_version,parser_revision,options_hash) DO NOTHING RETURNING id")
 .bind(input.source_id).bind(input.content_version).bind(&input.input_sha256).bind(parser_revision).bind(options_hash(options_revision)).bind(lease.job_id).bind(sqlx::types::Json(&document.blocks)).bind(execution_id).fetch_optional(&mut **tx).await?;
    if let Some(id) = inserted {
        sqlx::query("INSERT INTO outbox(job_id,topic,payload) VALUES($1,'file.extracted',$2)").bind(lease.job_id).bind(json!({"extraction_id":id,"source_id":input.source_id,"source_version":input.content_version})).execute(&mut **tx).await?;
    }
    lock_lease(tx, lease).await?;
    Ok(())
}

pub const SUPERVISION_REVISION: &str = "guardian-exit-v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceAttempt {
    pub attempt: i32,
    pub lease_token: Uuid,
    pub protocol_revision: String,
    pub request: crate::intelligence::ExecutionRequest,
}
/// Persist before POST while the exact live job lease is locked. A network
/// retry reuses the first request, including its original deadline.
pub async fn register_attempt(
    pool: &sqlx::PgPool,
    lease: &Lease,
    request: crate::intelligence::ExecutionRequest,
) -> Result<IntelligenceAttempt, DomainError> {
    let mut tx = pool.begin().await?;
    lock_lease(&mut tx, lease).await?;
    let mut payload: Value =
        sqlx::query_scalar("SELECT payload FROM jobs WHERE id=$1 AND kind='file.extract'")
            .bind(lease.job_id)
            .fetch_one(&mut *tx)
            .await?;
    let mut attempts = decode_attempts(&payload)?;
    if let Some(existing) = attempts.iter().find(|a| a.lease_token == lease.token) {
        return Ok(existing.clone());
    }
    let entry = IntelligenceAttempt {
        attempt: lease.attempt,
        lease_token: lease.token,
        protocol_revision: SUPERVISION_REVISION.into(),
        request,
    };
    attempts.push(entry.clone());
    payload["intelligence_attempts"] = json!(attempts);
    sqlx::query("UPDATE jobs SET payload=$2 WHERE id=$1")
        .bind(lease.job_id)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(entry)
}
fn decode_attempts(payload: &Value) -> Result<Vec<IntelligenceAttempt>, DomainError> {
    let entries: Vec<IntelligenceAttempt> = match payload.get("intelligence_attempts") {
        None => Vec::new(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|_| DomainError::Stale)?,
    };
    if entries
        .iter()
        .any(|a| a.protocol_revision != SUPERVISION_REVISION)
    {
        return Err(DomainError::Stale);
    }
    Ok(entries)
}
pub async fn load_attempts(
    pool: &sqlx::PgPool,
    job_id: Uuid,
) -> Result<Vec<IntelligenceAttempt>, DomainError> {
    let payload: Value = sqlx::query_scalar("SELECT payload FROM jobs WHERE id=$1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;
    decode_attempts(&payload)
}

/// Python publication retains the exact same source/lease/observation fence.
pub async fn publish_python(
    ctx: &AppContext,
    lease: &Lease,
    input: &ExtractionInput,
    document: &ExtractedDocument,
    attempt: &IntelligenceAttempt,
) -> Result<(), DomainError> {
    if !ctx.files().enabled || document.truncated {
        return Err(DomainError::Stale);
    }
    let mut tx = ctx.pool().begin().await?;
    publish_inner(ctx, &mut tx, lease, input, document, Some(attempt)).await?;
    tx.commit().await?;
    Ok(())
}
