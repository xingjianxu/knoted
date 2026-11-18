//! Full-text publication into Meilisearch.
//!
//! The domain tables stay the source of truth. [`publish_fulltext`] writes one
//! derived, searchable projection per source version, the publication ledger row
//! and the `fulltext.publish` job in a single transaction; [`jobs::enqueue`]
//! writes the matching outbox event, so an event never exists without its job.
//!
//! Publication is confirmed, not fire-and-forget:
//!
//! 1. `pending` — the projection and job exist, the engine has not answered yet;
//! 2. `submitted` — the engine accepted the request and its task id is stored.
//!    An accepted task is **not** a published document;
//! 3. `published` — the engine task reported `succeeded`; `published_at` is
//!    written in the same statement, so the two can never disagree;
//! 4. `superseded` / `failed` — terminal for this version.
//!
//! Every document key is derived from `<type>_<uuid>_v<version>`, so a retry is
//! an upsert of the one document and never a second visible item. Older versions
//! are removed by a separate `fulltext.cleanup` job; a read path must never rely
//! on how fast that deletion reaches the engine (T54 re-checks the current
//! version against PostgreSQL).

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::SearchConfig,
    context::AppContext,
    jobs::{self, ExecutionOutcome, Job, Lease},
};

/// Job kind that publishes one projection document.
pub const PUBLISH_JOB_KIND: &str = "fulltext.publish";
/// Job kind that removes superseded versions of one source from one index.
pub const CLEANUP_JOB_KIND: &str = "fulltext.cleanup";
/// The only engine this ledger knows; a new engine gets its own card.
pub const ENGINE: &str = "meilisearch";

const MAX_TITLE_BYTES: usize = 2048;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_TAGS: usize = 64;
const MAX_TAG_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
const MAX_FORMAT_BYTES: usize = 32;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const TASK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TASK_POLL_DEADLINE: Duration = Duration::from_secs(5);

/// Searchable attributes, most important first.
const SEARCHABLE_ATTRIBUTES: &[&str] = &["title", "body"];
/// Fields the unified search filters on (T54).
const FILTERABLE_ATTRIBUTES: &[&str] = &[
    "source_type",
    "source_id",
    "source_version",
    "project_id",
    "tags",
    "archived",
    "deleted",
    "path",
    "format",
];
/// Sort keys; `version` mirrors `source_version` with a sort-friendly name.
const SORTABLE_ATTRIBUTES: &[&str] = &["updated_at", "version"];

/// A kind of searchable source. The spellings match the shared source registry
/// (`chat_message`, not `chat`) so T54/T57 need no translation layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    File,
    Note,
    Project,
    Task,
    ChatMessage,
}

impl SourceType {
    /// Database and document-key spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceType::File => "file",
            SourceType::Note => "note",
            SourceType::Project => "project",
            SourceType::Task => "task",
            SourceType::ChatMessage => "chat_message",
        }
    }

    /// Inverse of [`SourceType::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "file" => SourceType::File,
            "note" => SourceType::Note,
            "project" => SourceType::Project,
            "task" => SourceType::Task,
            "chat_message" => SourceType::ChatMessage,
            _ => return None,
        })
    }
}

/// The unified searchable projection of one source version.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDocument {
    pub source_type: SourceType,
    pub source_id: Uuid,
    /// Content version of the source; it is part of the document key.
    pub source_version: i64,
    pub title: String,
    pub body: String,
    pub project_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub archived: bool,
    /// Derived deletion flag. It never replaces the read-time check against the
    /// domain row; it only keeps obviously stale hits out of a query.
    pub deleted: bool,
    pub path: Option<String>,
    pub format: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Publication state of one document in one index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Pending,
    Submitted,
    Published,
    Superseded,
    Failed,
}

impl PublicationStatus {
    /// Database spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            PublicationStatus::Pending => "pending",
            PublicationStatus::Submitted => "submitted",
            PublicationStatus::Published => "published",
            PublicationStatus::Superseded => "superseded",
            PublicationStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => PublicationStatus::Pending,
            "submitted" => PublicationStatus::Submitted,
            "published" => PublicationStatus::Published,
            "superseded" => PublicationStatus::Superseded,
            "failed" => PublicationStatus::Failed,
            _ => return None,
        })
    }
}

/// Publication ledger row as returned to callers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Publication {
    pub id: Uuid,
    pub document_id: Uuid,
    pub document_key: String,
    pub index_uid: String,
    pub status: PublicationStatus,
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub source_version: i64,
    pub meili_task_uid: Option<i64>,
    pub last_error: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

/// Input of [`get_publication`]. `source_version = None` reads the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetPublication {
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub source_version: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("full-text search is not configured")]
    Disabled,
    #[error("the document is older than the current version of its source")]
    Stale,
    #[error("invalid search document: {0}")]
    Invalid(&'static str),
    #[error("search engine unavailable")]
    Unavailable,
    #[error("search engine rejected the request: {0}")]
    Rejected(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Queue(#[from] crate::db::DbError),
}

/// Deterministic Meilisearch document id for a source version.
///
/// Meilisearch only accepts alphanumerics, `-` and `_` in a document id, so the
/// UUID is written without hyphens. Including the version means every version
/// owns exactly one key and a retry can only upsert that key.
pub fn document_key(source_type: SourceType, source_id: Uuid, source_version: i64) -> String {
    format!(
        "{}_{}_v{}",
        source_type.as_str(),
        source_id.simple(),
        source_version
    )
}

fn validate(document: &SourceDocument) -> Result<(), DomainError> {
    if document.source_version <= 0 {
        return Err(DomainError::Invalid("source_version must be positive"));
    }
    if document.title.trim().is_empty() {
        return Err(DomainError::Invalid("title must not be empty"));
    }
    if document.title.len() > MAX_TITLE_BYTES {
        return Err(DomainError::Invalid("title is too large"));
    }
    if document.body.len() > MAX_BODY_BYTES {
        return Err(DomainError::Invalid("body is too large"));
    }
    if document.tags.len() > MAX_TAGS
        || document
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag.len() > MAX_TAG_BYTES)
    {
        return Err(DomainError::Invalid("tags are invalid"));
    }
    if document
        .path
        .as_ref()
        .is_some_and(|path| path.len() > MAX_PATH_BYTES)
    {
        return Err(DomainError::Invalid("path is too large"));
    }
    if document
        .format
        .as_ref()
        .is_some_and(|format| format.len() > MAX_FORMAT_BYTES)
    {
        return Err(DomainError::Invalid("format is too large"));
    }
    Ok(())
}

/// Same advisory key for every publish/read of one source, so two versions of
/// the same source cannot both become current.
async fn lock_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_type: SourceType,
    source_id: Uuid,
) -> Result<(), DomainError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema() || ':' || $1, 0))")
        .bind(format!("{}:{}", source_type.as_str(), source_id))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Projects `document` into the search index and enqueues its publication.
///
/// Idempotent per `(source_type, source_id, source_version)`: the same version
/// returns the existing publication without a second job. An older version is
/// rejected; a newer version supersedes the current row. A previously failed
/// publication of the same version is retried with a new job.
///
/// The version identifies the projected content: a metadata-only change
/// (`title`, `body`, `tags`, `archived`, `deleted`, `path`, `format`,
/// `updated_at`) must arrive as a new `source_version`. Re-sending an existing
/// version keeps the stored projection.
pub async fn publish_fulltext(
    ctx: &AppContext,
    document: SourceDocument,
) -> Result<Publication, DomainError> {
    let config = ctx.search();
    if !config.enabled {
        return Err(DomainError::Disabled);
    }
    validate(&document)?;
    let key = document_key(
        document.source_type,
        document.source_id,
        document.source_version,
    );
    let mut tx = ctx.pool().begin().await?;
    lock_source(&mut tx, document.source_type, document.source_id).await?;

    let current: Option<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, source_version FROM source_documents \
         WHERE source_type=$1 AND source_id=$2 AND superseded_at IS NULL",
    )
    .bind(document.source_type.as_str())
    .bind(document.source_id)
    .fetch_optional(&mut *tx)
    .await?;

    let document_id = match current {
        Some((_id, version)) if version > document.source_version => {
            return Err(DomainError::Stale);
        }
        Some((id, version)) if version == document.source_version => id,
        Some((id, _)) => {
            // The new version wins immediately: readers re-check the current
            // version, so the old engine document may be removed lazily.
            sqlx::query(
                "UPDATE source_documents SET superseded_at=now() \
                 WHERE id=$1 AND superseded_at IS NULL",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            insert_document(&mut tx, &document).await?
        }
        None => insert_document(&mut tx, &document).await?,
    };

    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, status FROM fulltext_publications \
         WHERE engine=$1 AND index_uid=$2 AND document_key=$3",
    )
    .bind(ENGINE)
    .bind(&config.index_uid)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;

    let (publication_id, needs_job) = match existing {
        Some((id, status)) => {
            // A terminal owning job (failed, cancelled, or a success whose
            // ledger write was rolled back) must not block new work. While the
            // job is still queued/running the publish call stays idempotent.
            let retry = status == PublicationStatus::Failed.as_str()
                || (status != PublicationStatus::Published.as_str()
                    && owning_job_terminal(&mut tx, id).await?);
            if retry {
                sqlx::query(
                    "UPDATE fulltext_publications SET status='pending', meili_task_uid=NULL, \
                     last_error=NULL, attempts=attempts+1, updated_at=now() WHERE id=$1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            (id, retry)
        }
        None => {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO fulltext_publications (document_id, engine, index_uid, document_key) \
                 VALUES ($1,$2,$3,$4) RETURNING id",
            )
            .bind(document_id)
            .bind(ENGINE)
            .bind(&config.index_uid)
            .bind(&key)
            .fetch_one(&mut *tx)
            .await?;
            (id, true)
        }
    };

    if needs_job {
        let mut spec = jobs::JobSpec::new(PUBLISH_JOB_KIND);
        // The attempt counter keeps a manual republish of a failed version from
        // colliding with the job that already failed; the key still identifies
        // the publication and cannot create a second one.
        let attempts: i32 =
            sqlx::query_scalar("SELECT attempts FROM fulltext_publications WHERE id=$1")
                .bind(publication_id)
                .fetch_one(&mut *tx)
                .await?;
        spec.idempotency_key = Some(format!("{key}:{}:{attempts}", config.index_uid));
        spec.payload = json!({ "publication_id": publication_id });
        let outcome = jobs::enqueue(&mut tx, spec).await?;
        sqlx::query("UPDATE fulltext_publications SET job_id=$2, updated_at=now() WHERE id=$1")
            .bind(publication_id)
            .bind(outcome.job.id)
            .execute(&mut *tx)
            .await?;
    }

    let publication = load_publication_tx(&mut tx, publication_id).await?;
    tx.commit().await?;
    Ok(publication)
}

async fn insert_document(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document: &SourceDocument,
) -> Result<Uuid, DomainError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO source_documents (source_type, source_id, source_version, title, body, \
         project_id, tags, archived, deleted, path, format, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id",
    )
    .bind(document.source_type.as_str())
    .bind(document.source_id)
    .bind(document.source_version)
    .bind(&document.title)
    .bind(&document.body)
    .bind(document.project_id)
    .bind(&document.tags)
    .bind(document.archived)
    .bind(document.deleted)
    .bind(document.path.as_deref())
    .bind(document.format.as_deref())
    .bind(document.updated_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Whether the job that owns a publication already reached a terminal state, so
/// a new attempt is real work rather than a duplicate.
async fn owning_job_terminal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    publication_id: Uuid,
) -> Result<bool, DomainError> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT j.state FROM fulltext_publications p LEFT JOIN jobs j ON j.id=p.job_id \
         WHERE p.id=$1",
    )
    .bind(publication_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match row {
        Some((Some(state),)) => matches!(state.as_str(), "succeeded" | "failed" | "cancelled"),
        // No rows or no job: nothing will run, so new work must be enqueued.
        _ => true,
    })
}

/// Reads the publication of one source version, or its current version.
pub async fn get_publication(
    ctx: &AppContext,
    input: GetPublication,
) -> Result<Option<Publication>, DomainError> {
    let row: Option<PublicationRow> = sqlx::query_as(
        "SELECT p.id, p.document_id, p.document_key, p.index_uid, p.status, p.meili_task_uid, \
         p.last_error, p.published_at, d.source_type, d.source_id, d.source_version \
         FROM fulltext_publications p JOIN source_documents d ON d.id=p.document_id \
         WHERE d.source_type=$1 AND d.source_id=$2 AND ($3::bigint IS NULL OR d.source_version=$3) \
         AND p.index_uid=$4 \
         ORDER BY d.source_version DESC, p.created_at DESC LIMIT 1",
    )
    .bind(input.source_type.as_str())
    .bind(input.source_id)
    .bind(input.source_version)
    .bind(&ctx.search().index_uid)
    .fetch_optional(ctx.pool())
    .await?;
    Ok(row.and_then(PublicationRow::into_publication))
}

async fn load_publication_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<Publication, DomainError> {
    let row: PublicationRow = sqlx::query_as(
        "SELECT p.id, p.document_id, p.document_key, p.index_uid, p.status, p.meili_task_uid, \
         p.last_error, p.published_at, d.source_type, d.source_id, d.source_version \
         FROM fulltext_publications p JOIN source_documents d ON d.id=p.document_id \
         WHERE p.id=$1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    row.into_publication().ok_or(DomainError::Stale)
}

#[derive(sqlx::FromRow)]
struct PublicationRow {
    id: Uuid,
    document_id: Uuid,
    document_key: String,
    index_uid: String,
    status: String,
    meili_task_uid: Option<i64>,
    last_error: Option<String>,
    published_at: Option<DateTime<Utc>>,
    source_type: String,
    source_id: Uuid,
    source_version: i64,
}

impl PublicationRow {
    fn into_publication(self) -> Option<Publication> {
        Some(Publication {
            id: self.id,
            document_id: self.document_id,
            document_key: self.document_key,
            index_uid: self.index_uid,
            status: PublicationStatus::parse(&self.status)?,
            source_type: SourceType::parse(&self.source_type)?,
            source_id: self.source_id,
            source_version: self.source_version,
            meili_task_uid: self.meili_task_uid,
            last_error: self.last_error,
            published_at: self.published_at,
        })
    }
}

/// Everything the worker needs to submit a publication.
struct PublicationRecord {
    id: Uuid,
    job_id: Option<Uuid>,
    index_uid: String,
    document_key: String,
    status: String,
    meili_task_uid: Option<i64>,
    source_type: SourceType,
    source_id: Uuid,
    source_version: i64,
    superseded: bool,
    title: String,
    body: String,
    project_id: Option<Uuid>,
    tags: Vec<String>,
    archived: bool,
    deleted: bool,
    path: Option<String>,
    format: Option<String>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct PublicationRecordRow {
    id: Uuid,
    job_id: Option<Uuid>,
    index_uid: String,
    document_key: String,
    status: String,
    meili_task_uid: Option<i64>,
    source_type: String,
    source_id: Uuid,
    source_version: i64,
    superseded: bool,
    title: String,
    body: String,
    project_id: Option<Uuid>,
    tags: Vec<String>,
    archived: bool,
    deleted: bool,
    path: Option<String>,
    format: Option<String>,
    updated_at: DateTime<Utc>,
}

async fn load_record(
    pool: &PgPool,
    publication_id: Uuid,
) -> Result<Option<PublicationRecord>, sqlx::Error> {
    let row: Option<PublicationRecordRow> = sqlx::query_as(
        "SELECT p.id, p.job_id, p.index_uid, p.document_key, p.status, p.meili_task_uid, \
         d.source_type, d.source_id, d.source_version, d.superseded_at IS NOT NULL AS superseded, \
         d.title, d.body, d.project_id, d.tags, d.archived, d.deleted, d.path, d.format, d.updated_at \
         FROM fulltext_publications p JOIN source_documents d ON d.id=p.document_id \
         WHERE p.id=$1",
    )
    .bind(publication_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|row| {
        Some(PublicationRecord {
            id: row.id,
            job_id: row.job_id,
            index_uid: row.index_uid,
            document_key: row.document_key,
            status: row.status,
            meili_task_uid: row.meili_task_uid,
            source_type: SourceType::parse(&row.source_type)?,
            source_id: row.source_id,
            source_version: row.source_version,
            superseded: row.superseded,
            title: row.title,
            body: row.body,
            project_id: row.project_id,
            tags: row.tags,
            archived: row.archived,
            deleted: row.deleted,
            path: row.path,
            format: row.format,
            updated_at: row.updated_at,
        })
    }))
}

fn engine_document(record: &PublicationRecord) -> Value {
    json!({
        "id": record.document_key,
        "source_type": record.source_type.as_str(),
        "source_id": record.source_id,
        "source_version": record.source_version,
        "version": record.source_version,
        "title": record.title,
        "body": record.body,
        "project_id": record.project_id,
        "tags": record.tags,
        "archived": record.archived,
        "deleted": record.deleted,
        "path": record.path,
        "format": record.format,
        "updated_at": record.updated_at.timestamp(),
    })
}

/// Task state as reported by Meilisearch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineTask {
    Enqueued,
    Processing,
    Succeeded,
    Failed { message: String },
    Canceled,
}

impl EngineTask {
    /// Whether the task reached a state the caller can act on.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            EngineTask::Succeeded | EngineTask::Failed { .. } | EngineTask::Canceled
        )
    }
}

/// Minimal Meilisearch HTTP client: enough for index preparation, document
/// upserts, task polling and deletion. It is not a general-purpose SDK.
#[derive(Clone)]
pub struct MeiliClient {
    base: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl std::fmt::Debug for MeiliClient {
    // The API key is a credential: `{:?}` must never reveal it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeiliClient")
            .field("base", &self.base)
            .field("api_key_set", &self.api_key.is_some())
            .finish_non_exhaustive()
    }
}

impl MeiliClient {
    /// Builds a client for `config`; `api_key` comes from the credential file.
    pub fn new(config: &SearchConfig, api_key: Option<&str>) -> Result<MeiliClient, DomainError> {
        if !config.enabled {
            return Err(DomainError::Disabled);
        }
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| DomainError::Unavailable)?;
        Ok(MeiliClient {
            base: config.base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.map(str::to_owned),
            http,
        })
    }

    /// Builds a client from the process context.
    pub fn from_context(ctx: &AppContext) -> Result<MeiliClient, DomainError> {
        MeiliClient::new(ctx.search(), ctx.search_api_key())
    }

    pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let builder = self.http.request(method, format!("{}/{path}", self.base));
        match &self.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    pub(crate) async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<Value, DomainError> {
        let response = request.send().await.map_err(|_| DomainError::Unavailable)?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let message = body["message"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| body["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        if status.is_client_error() {
            Err(DomainError::Rejected(message))
        } else {
            Err(DomainError::Unavailable)
        }
    }

    /// Creates the index and its settings when they are missing.
    ///
    /// Settings are re-written only when they drifted, and the task of every
    /// write is waited for, so "the index exists" always means "the index can
    /// answer with the fields T54 filters and sorts on".
    pub async fn ensure_index(&self, uid: &str) -> Result<(), DomainError> {
        self.ensure_index_settings(uid, &engine_settings()).await
    }

    pub(crate) async fn ensure_index_settings(
        &self,
        uid: &str,
        expected: &Value,
    ) -> Result<(), DomainError> {
        let response = self
            .request(reqwest::Method::GET, &format!("indexes/{uid}"))
            .send()
            .await
            .map_err(|_| DomainError::Unavailable)?;
        let status = response.status();
        if status.as_u16() == 404 {
            // A concurrent publication may create the index first; losing that
            // race is not an error.
            let created = self
                .request(reqwest::Method::POST, "indexes")
                .json(&json!({ "uid": uid, "primaryKey": "id" }))
                .send()
                .await
                .map_err(|_| DomainError::Unavailable)?;
            let created_status = created.status();
            let body: Value = created.json().await.unwrap_or(Value::Null);
            if created_status.is_success() {
                self.require_task(task_uid(&body)?).await?;
            } else if created_status.as_u16() != 409 {
                return Err(DomainError::Unavailable);
            }
        } else if status.is_client_error() {
            return Err(DomainError::Rejected(format!(
                "engine index probe: HTTP {}",
                status.as_u16()
            )));
        } else if !status.is_success() {
            return Err(DomainError::Unavailable);
        }

        let response = self
            .request(reqwest::Method::GET, &format!("indexes/{uid}/settings"))
            .send()
            .await
            .map_err(|_| DomainError::Unavailable)?;
        let status = response.status();
        let settings: Value = response.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 404 {
            // Another publication may still be creating this index.
            return Err(DomainError::Unavailable);
        }
        if status.is_client_error() {
            return Err(DomainError::Rejected(format!(
                "engine settings probe: HTTP {}",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(DomainError::Unavailable);
        }
        if !settings_match(&settings, expected) {
            let body = self
                .send(
                    self.request(reqwest::Method::PATCH, &format!("indexes/{uid}/settings"))
                        .json(expected),
                )
                .await?;
            self.require_task(task_uid(&body)?).await?;
        }
        Ok(())
    }

    /// Waits for a task and maps a still-running or failed task to an error.
    async fn require_task(&self, uid: i64) -> Result<(), DomainError> {
        match self.wait_task(uid).await? {
            EngineTask::Succeeded => Ok(()),
            EngineTask::Failed { message } => Err(DomainError::Rejected(message)),
            EngineTask::Canceled => Err(DomainError::Rejected("engine task canceled".into())),
            EngineTask::Enqueued | EngineTask::Processing => Err(DomainError::Unavailable),
        }
    }

    /// Upserts one document and returns the engine task id.
    pub async fn submit_document(&self, uid: &str, document: Value) -> Result<i64, DomainError> {
        self.submit_documents(uid, &[document]).await
    }

    pub(crate) async fn submit_documents(
        &self,
        uid: &str,
        documents: &[Value],
    ) -> Result<i64, DomainError> {
        let body = self
            .send(
                self.request(reqwest::Method::PUT, &format!("indexes/{uid}/documents"))
                    .json(documents),
            )
            .await?;
        task_uid(&body)
    }

    /// Deletes documents by key; deleting an absent key is a no-op in the
    /// engine, which makes a cleanup retry safe.
    pub async fn delete_documents(&self, uid: &str, keys: &[String]) -> Result<i64, DomainError> {
        let body = self
            .send(
                self.request(
                    reqwest::Method::POST,
                    &format!("indexes/{uid}/documents/delete-batch"),
                )
                .json(&keys),
            )
            .await?;
        task_uid(&body)
    }

    /// Reads one task once.
    pub async fn task(&self, uid: i64) -> Result<EngineTask, DomainError> {
        let body = self
            .send(self.request(reqwest::Method::GET, &format!("tasks/{uid}")))
            .await?;
        Ok(EngineTask::from_status(body["status"].as_str(), &body))
    }

    /// Polls a task until it is terminal or the poll deadline passes.
    ///
    /// A still-running task is returned as [`EngineTask::Enqueued`]/
    /// [`EngineTask::Processing`], never as success.
    pub async fn wait_task(&self, uid: i64) -> Result<EngineTask, DomainError> {
        let deadline = tokio::time::Instant::now() + TASK_POLL_DEADLINE;
        loop {
            let task = self.task(uid).await?;
            if task.is_terminal() || tokio::time::Instant::now() >= deadline {
                return Ok(task);
            }
            tokio::time::sleep(TASK_POLL_INTERVAL).await;
        }
    }
}

impl EngineTask {
    fn from_status(status: Option<&str>, body: &Value) -> EngineTask {
        match status {
            Some("succeeded") => EngineTask::Succeeded,
            Some("failed") => EngineTask::Failed {
                message: body["error"]["message"]
                    .as_str()
                    .unwrap_or("engine task failed")
                    .to_owned(),
            },
            Some("canceled") | Some("cancelled") => EngineTask::Canceled,
            Some("processing") => EngineTask::Processing,
            _ => EngineTask::Enqueued,
        }
    }
}

fn task_uid(body: &Value) -> Result<i64, DomainError> {
    body["taskUid"]
        .as_i64()
        .ok_or_else(|| DomainError::Rejected("engine did not return a task id".into()))
}

fn engine_settings() -> Value {
    json!({
        "searchableAttributes": SEARCHABLE_ATTRIBUTES,
        "filterableAttributes": FILTERABLE_ATTRIBUTES,
        "sortableAttributes": SORTABLE_ATTRIBUTES,
    })
}

/// Whether the engine already carries the fields this protocol depends on.
fn settings_match(settings: &Value, expected: &Value) -> bool {
    expected.as_object().is_some_and(|fields| {
        fields.iter().all(|(field, value)| match value.as_array() {
            Some(wanted) => settings[field]
                .as_array()
                .is_some_and(|actual| wanted.iter().all(|element| actual.contains(element))),
            None => settings[field] == *value,
        })
    })
}

/// Runs one full-text job. Called by the worker dispatcher; tests call it
/// directly with a claimed lease.
pub async fn execute(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    if !ctx.search().enabled {
        return ExecutionOutcome::Cancelled;
    }
    match job.kind.as_str() {
        PUBLISH_JOB_KIND => execute_publish(ctx, lease, job).await,
        CLEANUP_JOB_KIND => execute_cleanup(ctx, lease, job).await,
        _ => ExecutionOutcome::PermanentFailure {
            message: "unsupported full-text job kind".into(),
        },
    }
}

async fn execute_publish(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    let Some(publication_id) = job
        .payload
        .get("publication_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return ExecutionOutcome::PermanentFailure {
            message: "invalid publication envelope".into(),
        };
    };
    match publish_once(ctx, lease, job, publication_id).await {
        Ok(()) => ExecutionOutcome::Succeeded,
        Err(error) => {
            let failure = outcome(error);
            // A permanent failure - and a retry budget that is now exhausted -
            // must end in the ledger's terminal `failed`. Otherwise the row
            // stays `pending`/`submitted` and the next publish call treats it
            // as work already in flight.
            let exhausted = matches!(failure, ExecutionOutcome::TemporaryFailure { .. })
                && job.attempts >= job.max_attempts;
            match &failure {
                ExecutionOutcome::PermanentFailure { message } => {
                    let _ = record_failed(ctx, lease, publication_id, message).await;
                }
                ExecutionOutcome::TemporaryFailure { message } if exhausted => {
                    let _ = record_failed(ctx, lease, publication_id, message).await;
                }
                _ => {}
            }
            failure
        }
    }
}

/// One publish attempt. Every error ends in [`outcome`]; the caller records the
/// terminal failure in the ledger.
async fn publish_once(
    ctx: &AppContext,
    lease: &Lease,
    job: &Job,
    publication_id: Uuid,
) -> Result<(), DomainError> {
    let client = MeiliClient::from_context(ctx)?;
    let mut record = load_record(ctx.pool(), publication_id)
        .await?
        // The projection was cleaned up with an older superseded version: the
        // work is moot rather than broken, so the job is cancelled quietly.
        .ok_or(DomainError::Stale)?;
    if record.job_id != Some(lease.job_id) {
        rebind_publication(ctx, lease, job, &record).await?;
        record = load_record(ctx.pool(), publication_id)
            .await?
            .ok_or(DomainError::Stale)?;
    }
    match PublicationStatus::parse(&record.status) {
        // Replaying a finished publication is a no-op, not a second document.
        Some(PublicationStatus::Published) => return Ok(()),
        Some(PublicationStatus::Superseded) => {
            return mark_superseded(ctx, lease, publication_id).await;
        }
        _ => {}
    }
    if record.superseded {
        // A newer version already won; submitting this one would only create a
        // stale hit. The cleanup job removes whatever reached the engine.
        return mark_superseded(ctx, lease, publication_id).await;
    }
    client.ensure_index(&record.index_uid).await?;

    let task_uid = match record.meili_task_uid {
        Some(uid) => uid,
        None => {
            // A newer version may have won while the index was being prepared:
            // never submit a projection that stopped being current.
            if !still_current(ctx.pool(), publication_id).await? {
                return mark_superseded(ctx, lease, publication_id).await;
            }
            let uid = client
                .submit_document(&record.index_uid, engine_document(&record))
                .await?;
            // If the lease was lost after the engine accepted the document, the
            // id must not be attributed to this attempt; the next owner submits
            // the same key and the engine upserts it.
            record_submission(ctx, lease, publication_id, uid).await?;
            uid
        }
    };

    match client.wait_task(task_uid).await? {
        EngineTask::Succeeded => record_published(ctx, lease, publication_id).await,
        EngineTask::Failed { .. } | EngineTask::Canceled => Err(DomainError::Rejected(
            "engine task failed or was canceled".into(),
        )),
        EngineTask::Enqueued | EngineTask::Processing => Err(DomainError::Unavailable),
    }
}

/// A platform retry owns a new job and submits the same document key again.
async fn rebind_publication(
    ctx: &AppContext,
    lease: &Lease,
    job: &Job,
    record: &PublicationRecord,
) -> Result<(), DomainError> {
    let original = record.job_id.ok_or(DomainError::Stale)?;
    if job.retry_of != Some(original) {
        return Err(DomainError::Stale);
    }
    let updated = sqlx::query(
        "UPDATE fulltext_publications p SET job_id=$2, updated_at=now(), \
         status=CASE WHEN p.status IN ('published','superseded') THEN p.status ELSE 'pending' END, \
         meili_task_uid=CASE WHEN p.status IN ('published','superseded') THEN p.meili_task_uid END, \
         last_error=NULL \
         FROM jobs original, jobs retry \
         WHERE p.id=$1 AND p.job_id=$3 AND original.id=$3 \
         AND original.state IN ('failed','cancelled') \
         AND retry.id=$2 AND retry.lease_token=$4 AND retry.state='running' \
         AND retry.lease_expires_at>clock_timestamp()",
    )
    .bind(record.id)
    .bind(lease.job_id)
    .bind(original)
    .bind(lease.token)
    .execute(ctx.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DomainError::Stale);
    }
    Ok(())
}

/// Whether the projection is still the current version of its source.
async fn still_current(pool: &PgPool, publication_id: Uuid) -> Result<bool, sqlx::Error> {
    let current: Option<bool> = sqlx::query_scalar(
        "SELECT d.superseded_at IS NULL FROM fulltext_publications p \
         JOIN source_documents d ON d.id=p.document_id WHERE p.id=$1",
    )
    .bind(publication_id)
    .fetch_optional(pool)
    .await?;
    Ok(current == Some(true))
}

async fn execute_cleanup(ctx: &AppContext, lease: &Lease, job: &Job) -> ExecutionOutcome {
    let (Some(source_type), Some(source_id), Some(keep_version), Some(index_uid)) = (
        job.payload
            .get("source_type")
            .and_then(Value::as_str)
            .and_then(SourceType::parse),
        job.payload
            .get("source_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok()),
        job.payload.get("keep_version").and_then(Value::as_i64),
        job.payload.get("index_uid").and_then(Value::as_str),
    ) else {
        return ExecutionOutcome::PermanentFailure {
            message: "invalid cleanup envelope".into(),
        };
    };
    let keys = match cleanup_keys(ctx.pool(), source_type, source_id, keep_version).await {
        Ok(keys) => keys,
        Err(_) => return temporary("cleanup ledger unavailable"),
    };
    if !keys.is_empty() {
        let client = match MeiliClient::from_context(ctx) {
            Ok(client) => client,
            Err(error) => return outcome(error),
        };
        let task_uid = match client.delete_documents(index_uid, &keys).await {
            Ok(uid) => uid,
            Err(error) => return outcome(error),
        };
        match client.wait_task(task_uid).await {
            Ok(EngineTask::Succeeded) => {}
            Ok(EngineTask::Failed { .. } | EngineTask::Canceled) => {
                return ExecutionOutcome::PermanentFailure {
                    message: "search cleanup task failed".into(),
                };
            }
            Ok(EngineTask::Enqueued | EngineTask::Processing) => {
                return temporary("search cleanup not confirmed");
            }
            Err(error) => return outcome(error),
        }
    }
    match finish_cleanup(ctx, lease, source_type, source_id, keep_version).await {
        Ok(()) => ExecutionOutcome::Succeeded,
        Err(error) => outcome(error),
    }
}

/// Cancelling a publication only drops in-flight HTTP requests; nothing
/// survives the future, so the exit is always confirmed.
pub async fn cancel_and_wait(_ctx: &AppContext, _lease: &Lease) -> bool {
    true
}

/// Lease-fenced submission record: an accepted task id may only be stored by the
/// attempt that still owns the job.
async fn record_submission(
    ctx: &AppContext,
    lease: &Lease,
    publication_id: Uuid,
    task_uid: i64,
) -> Result<(), DomainError> {
    let updated = sqlx::query(
        "UPDATE fulltext_publications p SET status='submitted', meili_task_uid=$3, \
         attempts=p.attempts+1, updated_at=now() \
         FROM jobs j WHERE p.id=$1 AND p.job_id=j.id AND j.id=$2 AND j.lease_token=$4 \
         AND j.state='running' AND j.lease_expires_at>clock_timestamp()",
    )
    .bind(publication_id)
    .bind(lease.job_id)
    .bind(task_uid)
    .bind(lease.token)
    .execute(ctx.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DomainError::Stale);
    }
    Ok(())
}

/// Marks the document published and queues the cleanup of older versions.
async fn record_published(
    ctx: &AppContext,
    lease: &Lease,
    publication_id: Uuid,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let updated = sqlx::query(
        "UPDATE fulltext_publications p SET status='published', published_at=now(), \
         last_error=NULL, updated_at=now() \
         FROM jobs j WHERE p.id=$1 AND p.job_id=j.id AND j.id=$2 AND j.lease_token=$3 \
         AND j.state='running' AND j.lease_expires_at>clock_timestamp() \
         AND p.status IN ('pending','submitted')",
    )
    .bind(publication_id)
    .bind(lease.job_id)
    .bind(lease.token)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(DomainError::Stale);
    }
    let (source_type, source_id, source_version, index_uid): (String, Uuid, i64, String) =
        sqlx::query_as(
            "SELECT d.source_type, d.source_id, d.source_version, p.index_uid \
             FROM fulltext_publications p JOIN source_documents d ON d.id=p.document_id \
             WHERE p.id=$1",
        )
        .bind(publication_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE fulltext_publications p SET status='superseded', updated_at=now() \
         FROM source_documents d \
         WHERE p.document_id=d.id AND d.source_type=$1 AND d.source_id=$2 \
         AND d.source_version<$3 AND p.engine=$4 AND p.index_uid=$5 \
         AND p.status IN ('pending','submitted','published')",
    )
    .bind(&source_type)
    .bind(source_id)
    .bind(source_version)
    .bind(ENGINE)
    .bind(&index_uid)
    .execute(&mut *tx)
    .await?;
    // Cleanup is keyed by the version that must survive, so publishing v3 keeps
    // v3 and removes everything older; it is idempotent and may run late. The
    // first version of a source has nothing older to remove.
    let has_older: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM source_documents \
         WHERE source_type=$1 AND source_id=$2 AND source_version<$3)",
    )
    .bind(&source_type)
    .bind(source_id)
    .bind(source_version)
    .fetch_one(&mut *tx)
    .await?;
    if has_older {
        let mut spec = jobs::JobSpec::new(CLEANUP_JOB_KIND);
        spec.idempotency_key = Some(format!(
            "{source_type}:{source_id}:{source_version}:{index_uid}"
        ));
        spec.payload = json!({
            "source_type": source_type,
            "source_id": source_id,
            "keep_version": source_version,
            "index_uid": index_uid,
        });
        jobs::enqueue(&mut tx, spec).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn mark_superseded(
    ctx: &AppContext,
    lease: &Lease,
    publication_id: Uuid,
) -> Result<(), DomainError> {
    let updated = sqlx::query(
        "UPDATE fulltext_publications p SET status='superseded', updated_at=now() \
         FROM jobs j WHERE p.id=$1 AND p.job_id=j.id AND j.id=$2 AND j.lease_token=$3 \
         AND j.state='running' AND p.status <> 'published'",
    )
    .bind(publication_id)
    .bind(lease.job_id)
    .bind(lease.token)
    .execute(ctx.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DomainError::Stale);
    }
    Ok(())
}

async fn record_failed(
    ctx: &AppContext,
    lease: &Lease,
    publication_id: Uuid,
    message: &str,
) -> Result<(), DomainError> {
    let updated = sqlx::query(
        "UPDATE fulltext_publications p SET status='failed', last_error=$4, updated_at=now() \
         FROM jobs j WHERE p.id=$1 AND p.job_id=j.id AND j.id=$2 AND j.lease_token=$3 \
         AND j.state='running'",
    )
    .bind(publication_id)
    .bind(lease.job_id)
    .bind(lease.token)
    .bind(message)
    .execute(ctx.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DomainError::Stale);
    }
    Ok(())
}

/// Keys of every older version of one source.
///
/// Derived from the projection rows instead of the ledger's task id: a
/// submission the engine accepted before its acknowledgement was lost has no
/// task id recorded, but its document still exists and must be deleted. Deleting
/// an absent key is a no-op in the engine, so over-listing is safe.
async fn cleanup_keys(
    pool: &PgPool,
    source_type: SourceType,
    source_id: Uuid,
    keep_version: i64,
) -> Result<Vec<String>, sqlx::Error> {
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT source_version FROM source_documents \
         WHERE source_type=$1 AND source_id=$2 AND source_version<$3 \
         ORDER BY source_version",
    )
    .bind(source_type.as_str())
    .bind(source_id)
    .bind(keep_version)
    .fetch_all(pool)
    .await?;
    Ok(versions
        .into_iter()
        .map(|version| document_key(source_type, source_id, version))
        .collect())
}

/// Drops the superseded projections and their publication rows once the engine
/// confirmed the deletion.
async fn finish_cleanup(
    ctx: &AppContext,
    lease: &Lease,
    source_type: SourceType,
    source_id: Uuid,
    keep_version: i64,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    let live: Option<bool> = sqlx::query_scalar(
        "SELECT state='running' AND lease_token=$2 AND lease_expires_at>clock_timestamp() \
         FROM jobs WHERE id=$1",
    )
    .bind(lease.job_id)
    .bind(lease.token)
    .fetch_optional(&mut *tx)
    .await?;
    if live != Some(true) {
        tx.rollback().await?;
        return Err(DomainError::Stale);
    }
    sqlx::query(
        "DELETE FROM source_documents \
         WHERE source_type=$1 AND source_id=$2 AND source_version<$3",
    )
    .bind(source_type.as_str())
    .bind(source_id)
    .bind(keep_version)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn temporary(message: &'static str) -> ExecutionOutcome {
    ExecutionOutcome::TemporaryFailure {
        message: message.into(),
    }
}

fn outcome(error: DomainError) -> ExecutionOutcome {
    match error {
        DomainError::Stale => ExecutionOutcome::Cancelled,
        DomainError::Disabled => ExecutionOutcome::Cancelled,
        DomainError::Invalid(message) => ExecutionOutcome::PermanentFailure {
            message: format!("invalid search document: {message}"),
        },
        DomainError::Rejected(_) => ExecutionOutcome::PermanentFailure {
            message: "search engine rejected the publication".into(),
        },
        DomainError::Unavailable | DomainError::Database(_) | DomainError::Queue(_) => {
            temporary("search publication unavailable")
        }
    }
}
