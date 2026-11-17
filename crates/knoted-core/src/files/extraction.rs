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
fn options_hash() -> String {
    Sha256Hex::digest(text::OPTIONS_REVISION.as_bytes()).to_string()
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
    if !row.is_some_and(|r| text::supports(&r.0)) {
        return Ok(());
    }
    let input:ExtractionInput=sqlx::query_as("SELECT id AS source_id,root_id,content_version,sha256 AS input_sha256,input_object_id,size,observation_version,captured_scan_id,observation FROM file_sources WHERE id=$1").bind(source_id).fetch_one(&mut **tx).await?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM file_extractions WHERE source_id=$1 AND content_version=$2 AND parser_revision=$3 AND options_hash=$4)").bind(source_id).bind(input.content_version).bind(text::PARSER_REVISION).bind(options_hash()).fetch_one(&mut **tx).await?;
    if exists {
        return Ok(());
    }
    let mut spec = jobs::JobSpec::new(EXTRACT_JOB_KIND);
    // A newly captured observation can recover from a stale terminal attempt;
    // replay of the same receipt cannot create an unbounded retry loop.
    spec.idempotency_key = Some(format!(
        "{}:{}:{}:{}:{}:{}",
        source_id,
        input.content_version,
        input.observation_version,
        input.captured_scan_id,
        text::PARSER_REVISION,
        options_hash()
    ));
    spec.payload = json!(input);
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
    let result = publish_inner(ctx, &mut tx, lease, input, document).await;
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
) -> Result<(), DomainError> {
    lock_lease(tx, lease).await?;
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
    let inserted:Option<Uuid>=sqlx::query_scalar("INSERT INTO file_extractions(source_id,content_version,input_sha256,parser_revision,options_hash,job_id,blocks) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(source_id,content_version,parser_revision,options_hash) DO NOTHING RETURNING id")
 .bind(input.source_id).bind(input.content_version).bind(&input.input_sha256).bind(text::PARSER_REVISION).bind(options_hash()).bind(lease.job_id).bind(sqlx::types::Json(&document.blocks)).fetch_optional(&mut **tx).await?;
    if let Some(id) = inserted {
        sqlx::query("INSERT INTO outbox(job_id,topic,payload) VALUES($1,'file.extracted',$2)").bind(lease.job_id).bind(json!({"extraction_id":id,"source_id":input.source_id,"source_version":input.content_version})).execute(&mut **tx).await?;
    }
    lock_lease(tx, lease).await?;
    Ok(())
}
