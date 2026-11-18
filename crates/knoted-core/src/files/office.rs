//! Office extraction identity and ephemeral, fixed-version preview authorization.
use super::{artifact, download, pdf::Error, text::ExtractedDocument};
use crate::{
    config::{AuthConfig, OfficeConfig, is_canonical_https_origin},
    context::AppContext,
    intelligence::{ExecutionRequest, ExecutionResult},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    time::Instant,
};
use uuid::Uuid;
pub const PARSER_REVISION: &str = "office-ooxml-body-v2-hidden-slides-included-cache-only";
pub const OPTIONS_REVISION: &str = "office-ooxml-body-v2-hidden-slides-included-cache-only:66f5d3305d4cda1ad981b12e95d75f4c4f4efcc31e5ca45a8985ead92f65da8a";
// The source extension is a durable selector: renames cannot reuse a success
// (or exhausted failure budget) produced for a different expected format.
pub const RETIRED_DOCX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:docx";
pub const DOCX_PARSER_REVISION: &str = "office-ooxml-body-v3:docx";
pub const XLSX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:xlsx";
pub const PPTX_PARSER_REVISION: &str =
    "office-ooxml-body-v2-hidden-slides-included-cache-only:pptx";
pub fn format_for_parser(parser: &str) -> Option<&'static str> {
    match parser {
        DOCX_PARSER_REVISION => Some("docx"),
        XLSX_PARSER_REVISION => Some("xlsx"),
        PPTX_PARSER_REVISION => Some("pptx"),
        _ => None,
    }
}
pub fn identity(parser: &str) -> Option<(&'static str, &'static str)> {
    [
        DOCX_PARSER_REVISION,
        XLSX_PARSER_REVISION,
        PPTX_PARSER_REVISION,
    ]
    .into_iter()
    .find(|p| *p == parser)
    .map(|p| (p, OPTIONS_REVISION))
}
pub fn identity_for_path(path: &str) -> Option<(&'static str, &'static str)> {
    let parser = match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "docx" => DOCX_PARSER_REVISION,
        "xlsx" => XLSX_PARSER_REVISION,
        "pptx" => PPTX_PARSER_REVISION,
        _ => return None,
    };
    identity(parser)
}
pub fn supports(path: &str) -> bool {
    identity_for_path(path).is_some()
}
fn parser_summary(source_format: &str) -> Value {
    let parser_revision = if source_format == "docx" {
        DOCX_PARSER_REVISION
    } else {
        PARSER_REVISION
    };
    json!({"parser_revision":parser_revision,"options_revision":OPTIONS_REVISION,
        "limits":{"members":4096,"part_bytes":33554432,"expanded_bytes":134217728,"compression_ratio":200,
        "xml_nodes":500000,"xml_depth":64,"text_bytes":16777216,"cells":100000,"blocks":100000}})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathLocation {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellLocation {
    sheet: String,
    cell: String,
    range: Option<String>,
    value_source: Option<String>,
}
fn coordinate(address: &str) -> Option<(u32, u32)> {
    let split = address.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = address.split_at(split);
    if letters.is_empty()
        || letters.len() > 3
        || digits.starts_with('0')
        || !digits.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut column = 0;
    for c in letters.bytes() {
        if !c.is_ascii_uppercase() {
            return None;
        }
        column = column * 26 + u32::from(c - b'A') + 1;
    }
    let row = digits.parse::<u32>().ok()?;
    (column <= 16384 && (1..=1048576).contains(&row)).then_some((column, row))
}
fn numbered(part: &str, name: &str) -> bool {
    part.strip_prefix(name)
        .and_then(|s| s.strip_prefix(':'))
        .is_some_and(|s| {
            !s.starts_with('0')
                && s.bytes().all(|b| b.is_ascii_digit())
                && s.parse::<u32>().is_ok_and(|n| n > 0)
        })
}
fn path_valid(path: &str, format: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    let mut index = 0;
    if format == "docx" {
        if parts.first() != Some(&"body") {
            return false;
        }
        index = 1;
    } else {
        while index < parts.len() && numbered(parts[index], "shape") {
            index += 1;
        }
        if index == 0 {
            return false;
        }
    }
    while index < parts.len() {
        if numbered(parts[index], "paragraph") {
            return index + 1 == parts.len();
        }
        let table = if format == "docx" {
            numbered(parts[index], "table")
        } else {
            parts[index] == "table"
        };
        if !table
            || index + 3 >= parts.len()
            || !numbered(parts[index + 1], "row")
            || !numbered(parts[index + 2], "cell")
        {
            return false;
        }
        index += 3;
        if format == "pptx" {
            return index + 1 == parts.len() && numbered(parts[index], "paragraph");
        }
    }
    false
}
fn location_valid(value: &Value, format: &str) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    if format != "xlsx" {
        return serde_json::from_str::<PathLocation>(text)
            .is_ok_and(|p| path_valid(&p.path, format));
    }
    let Ok(cell) = serde_json::from_str::<CellLocation>(text) else {
        return false;
    };
    let Some(start) = coordinate(&cell.cell) else {
        return false;
    };
    if cell.sheet.is_empty()
        || cell.sheet.chars().count() > 31
        || cell.sheet.contains('\0')
        || cell
            .value_source
            .as_deref()
            .is_some_and(|s| s != "formula_cache")
    {
        return false;
    }
    if let Some(range) = cell.range {
        let Some((first, last)) = range.split_once(':') else {
            return false;
        };
        if first != cell.cell
            || !coordinate(last).is_some_and(|end| end.0 >= start.0 && end.1 >= start.1)
        {
            return false;
        }
    }
    true
}
pub fn decode_artifact(
    bytes: &[u8],
    request: &ExecutionRequest,
) -> Result<ExtractedDocument, Error> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if v["kind"] != "parse"
        || v["input_sha256"] != request.input_sha256.as_str()
        || v["source_version"].as_i64() != Some(request.source_version)
        || request.options_revision != OPTIONS_REVISION
        || v["options_revision"] != OPTIONS_REVISION
        || v["truncated"] != false
        || !v["child_pid"].as_u64().is_some_and(|p| p > 0)
    {
        return Err(Error::Invalid);
    }
    let document: ExtractedDocument =
        serde_json::from_value(v.clone()).map_err(|_| Error::Invalid)?;
    if v["parser"] != parser_summary(&document.source_format) {
        return Err(Error::Invalid);
    }
    match document.source_format.as_str() {
        "docx" | "xlsx" if document.page_count.is_none() => {}
        "pptx" if document.page_count.is_some_and(|n| n <= 4096) => {}
        _ => return Err(Error::Invalid),
    }
    if document.blocks.len() > 100000 {
        return Err(Error::Invalid);
    }
    let mut characters = 0;
    let mut text_bytes = 0;
    for (ordinal, block) in document.blocks.iter().enumerate() {
        if block.ordinal != ordinal
            || block.kind != "paragraph"
            || block.text.trim().is_empty()
            || block.text.contains('\0')
            || block.bbox.is_some()
            || !location_valid(&block.structure, &document.source_format)
        {
            return Err(Error::Invalid);
        }
        if document.source_format == "pptx" {
            if !block
                .page
                .is_some_and(|page| page > 0 && page <= document.page_count.unwrap_or(0))
            {
                return Err(Error::Invalid);
            }
        } else if block.page.is_some() {
            return Err(Error::Invalid);
        }
        characters += block.text.chars().count();
        text_bytes += block.text.len();
    }
    if text_bytes > 16 * 1024 * 1024
        || v["text_length"].as_u64()
            != Some((characters + document.blocks.len().saturating_sub(1)) as u64)
    {
        return Err(Error::Invalid);
    }
    Ok(document)
}
pub async fn read_artifact(
    ctx: &AppContext,
    request: &ExecutionRequest,
    result: &ExecutionResult,
) -> Result<ExtractedDocument, Error> {
    let bytes = artifact::read_bytes(ctx, request, result).await?;
    decode_artifact(&bytes, request)
}

pub(super) const MAX_PREVIEW_BYTES: i64 = 50 * 1024 * 1024;
const PREVIEW_SLOTS: usize = 2;
const TTL: chrono::Duration = chrono::Duration::seconds(120);

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("file not found")]
    NotFound,
    #[error("selected content version is unavailable")]
    Conflict,
    #[error("invalid version or unsupported Office format or size")]
    Unsupported,
    #[error("Office preview capacity exhausted")]
    AtCapacity,
    #[error("Office preview unavailable")]
    Unavailable,
    #[error("session no longer valid")]
    SessionEnded,
}

impl From<download::DomainError> for DomainError {
    fn from(error: download::DomainError) -> Self {
        match error {
            download::DomainError::NotFound => Self::NotFound,
            download::DomainError::SelectionConflict => Self::Conflict,
            download::DomainError::UnsupportedPreview => Self::Unsupported,
            download::DomainError::Unavailable | download::DomainError::Database(_) => {
                Self::Unavailable
            }
        }
    }
}

pub struct CreatePreview {
    pub file_id: Uuid,
    pub expected_content_version: i64,
    pub session_id: Uuid,
}

pub struct RevokePreview {
    pub file_id: Uuid,
    pub preview_id: Uuid,
    pub session_id: Uuid,
}

pub struct GetPreviewSource<'a> {
    pub preview_id: Uuid,
    pub outbox_jwt: &'a str,
}

// These values deliberately do not implement Debug: the configuration holds
// the signed browser JWT, and the source contains private bytes.
#[derive(Serialize)]
pub struct OfficePreview {
    pub preview_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub document_server_origin: String,
    pub config: Value,
}

pub struct PreviewSource {
    pub mime: &'static str,
    pub bytes: Bytes,
}

struct Entry {
    file_id: Uuid,
    session_id: Uuid,
    expires_at: DateTime<Utc>,
    deadline: Instant,
    mime: &'static str,
    bytes: Bytes,
    // Dropping an entry (including revocation) wakes its cleanup task.
    _expiry_cancel: oneshot::Sender<()>,
}

impl Entry {
    fn live(&self, now: DateTime<Utc>) -> bool {
        now < self.expires_at && Instant::now() < self.deadline
    }
}

struct Snapshot {
    bytes: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}

impl AsRef<[u8]> for Snapshot {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

pub(crate) struct PreviewStore {
    entries: Mutex<HashMap<Uuid, Entry>>,
    permits: Arc<Semaphore>,
    document_server_origin: String,
    public_origin: String,
    signer: hmac::Key,
}

impl PreviewStore {
    pub(crate) fn new(config: &OfficeConfig, auth: &AuthConfig) -> Option<Self> {
        if config.validate().is_err() || !is_canonical_https_origin(&auth.origin) {
            return None;
        }
        let signer = match load_signer(&config.jwt_secret_file) {
            Some(signer) => signer,
            None => {
                tracing::warn!("Office preview disabled: signing credential unavailable");
                return None;
            }
        };
        Some(Self {
            entries: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(PREVIEW_SLOTS)),
            document_server_origin: config.document_server_origin.clone(),
            public_origin: auth.origin.clone(),
            signer,
        })
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<Uuid, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reap(&self, now: DateTime<Utc>) {
        self.entries().retain(|_, entry| entry.live(now));
    }

    fn source_url(&self, preview_id: Uuid) -> String {
        format!(
            "{}/api/v1/office-preview-sources/{preview_id}",
            self.public_origin
        )
    }
}

fn load_signer(path: &Path) -> Option<hmac::Key> {
    use rustix::fs::{Mode, OFlags, open};
    // A bad credential path must not block startup on a FIFO or allocate an
    // unbounded file. Managed symlinks to regular credential files are allowed.
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .ok()?;
    let file = File::from(fd);
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    const MAX_CREDENTIAL_BYTES: u64 = 4096;
    let mut secret = String::new();
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_string(&mut secret)
        .ok()?;
    let value = secret.trim_end_matches(['\r', '\n']);
    if secret.len() as u64 > MAX_CREDENTIAL_BYTES || value.trim().is_empty() {
        return None;
    }
    Some(hmac::Key::new(hmac::HMAC_SHA256, value.as_bytes()))
}

// Keep the session row shared-locked through the in-memory decision. A
// concurrent logout cannot commit between the lookup and that decision.
// Never renew the idle deadline or acquire the cache mutex across an await.
async fn with_session_lock<T>(
    ctx: &AppContext,
    session_id: Uuid,
    decide: impl FnOnce(Option<DateTime<Utc>>) -> T,
) -> Result<T, sqlx::Error> {
    let mut tx = ctx.pool().begin().await?;
    let decision = sqlx::query_scalar("SELECT LEAST(idle_expires_at, absolute_expires_at) FROM sessions WHERE id = $1 AND revoked_at IS NULL FOR SHARE")
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map(decide);
    // Release the row lock on the server before returning the response.
    tx.rollback().await?;
    decision
}

pub async fn create_preview(
    ctx: &AppContext,
    input: CreatePreview,
) -> Result<OfficePreview, DomainError> {
    let store = ctx.office().ok_or(DomainError::Unavailable)?;
    if input.expected_content_version < 1 {
        return Err(DomainError::Unsupported);
    }
    store.reap(ctx.now());
    let permit = store
        .permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| DomainError::AtCapacity)?;
    let download::Admitted {
        guard: permit,
        content,
    } = download::read_office_content(
        ctx,
        download::ReadOfficeContent {
            id: input.file_id,
            expected_content_version: input.expected_content_version,
        },
        permit,
    )
    .await?;
    let bytes = Bytes::from_owner(Snapshot {
        bytes: content.bytes,
        _permit: permit,
    });
    let Some(file_type) = identity_for_path(&content.name).and_then(|(p, _)| format_for_parser(p))
    else {
        return Err(DomainError::Unsupported);
    };
    let (document_type, mime) = match file_type {
        "docx" => (
            "word",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "xlsx" => (
            "cell",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "pptx" => (
            "slide",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        _ => return Err(DomainError::Unsupported),
    };

    let preview_id = Uuid::new_v4();
    let url = store.source_url(preview_id);
    with_session_lock(ctx, input.session_id, |session_deadline| {
        let now = ctx.now();
        if session_deadline.is_none_or(|deadline| deadline <= now) {
            return Err(DomainError::SessionEnded);
        }
        let expires_at = now + TTL;
        let deadline = Instant::now() + TTL.to_std().expect("positive Office preview TTL");
        let config = signed_config(
            store,
            preview_id,
            file_type,
            document_type,
            &content.name,
            &url,
            expires_at,
        )?;
        let (cancel, cancelled) = oneshot::channel();
        store.entries().insert(
            preview_id,
            Entry {
                file_id: input.file_id,
                session_id: input.session_id,
                expires_at,
                deadline,
                mime,
                bytes,
                _expiry_cancel: cancel,
            },
        );
        let weak_store = Arc::downgrade(store);
        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => {
                    if let Some(store) = weak_store.upgrade() {
                        store.entries().remove(&preview_id);
                    }
                }
                _ = cancelled => {}
            }
        });
        Ok(OfficePreview {
            preview_id,
            expires_at,
            document_server_origin: store.document_server_origin.clone(),
            config,
        })
    })
    .await
    .unwrap_or_else(|_| {
        store.entries().remove(&preview_id);
        Err(DomainError::Unavailable)
    })
}

fn signed_config(
    store: &PreviewStore,
    preview_id: Uuid,
    file_type: &str,
    document_type: &str,
    name: &str,
    url: &str,
    expires_at: DateTime<Utc>,
) -> Result<Value, DomainError> {
    let mut config = json!({
        "documentType": document_type,
        "document": {
            "fileType": file_type,
            "key": preview_id,
            "title": name,
            "url": url,
            "permissions": {
                "edit": false, "review": false, "comment": false, "fillForms": false,
                "modifyFilter": false, "modifyContentControl": false, "chat": false,
                "download": false, "print": false, "copy": false, "changeHistory": false,
                "protect": false
            }
        },
        "editorConfig": {
            "mode": "view",
            "customization": { "plugins": false, "macros": false, "macrosMode": "disable" }
        }
    });
    // ONLYOFFICE expects SDK fields as top-level JWT claims. The source
    // expiry is independently enforced even if Docs has already seen the JWT.
    config["exp"] = json!(expires_at.timestamp());
    let payload = serde_json::to_vec(&config).map_err(|_| DomainError::Unavailable)?;
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let tag = hmac::sign(&store.signer, input.as_bytes());
    let signed = format!("{input}.{}", URL_SAFE_NO_PAD.encode(tag));
    if let Value::Object(fields) = &mut config {
        fields.remove("exp");
        fields.insert("token".into(), Value::String(signed));
    }
    Ok(config)
}

pub fn revoke_preview(ctx: &AppContext, input: RevokePreview) {
    if let Some(store) = ctx.office() {
        let mut entries = store.entries();
        if entries.get(&input.preview_id).is_some_and(|entry| {
            entry.file_id == input.file_id && entry.session_id == input.session_id
        }) {
            entries.remove(&input.preview_id);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboxJwtHeader<'a> {
    alg: &'a str,
    typ: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboxClaims {
    payload: OutboxPayload,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboxPayload {
    url: String,
}

impl OutboxClaims {
    fn live(&self, now: DateTime<Utc>) -> bool {
        0 < self.iat && self.iat <= now.timestamp() && now.timestamp() < self.exp
    }
}

fn verify_outbox(key: &hmac::Key, jwt: &str) -> Option<OutboxClaims> {
    const MAX_JWT_BYTES: usize = 4096;
    if jwt.len() > MAX_JWT_BYTES {
        return None;
    }
    let (signed, tag) = jwt.rsplit_once('.')?;
    let (header, payload) = signed.split_once('.')?;
    // The no-pad engine also rejects invalid alphabet and non-zero unused
    // bits; authenticate the original compact encoding, never reserialize it.
    let header = URL_SAFE_NO_PAD.decode(header).ok()?;
    let header: OutboxJwtHeader<'_> = serde_json::from_slice(&header).ok()?;
    if header.alg != "HS256" || header.typ.is_some_and(|typ| typ != "JWT") {
        return None;
    }
    let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
    hmac::verify(key, signed.as_bytes(), &tag).ok()?;
    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    // The official converter wraps the URL in payload and adds iat/exp.
    // Browser configs use a different claim shape and are not outbox claims.
    serde_json::from_slice(&payload).ok()
}

pub async fn get_preview_source(
    ctx: &AppContext,
    input: GetPreviewSource<'_>,
) -> Option<PreviewSource> {
    let store = ctx.office()?;
    let claims = verify_outbox(&store.signer, input.outbox_jwt)?;
    let now = ctx.now();
    if !claims.live(now) || claims.payload.url != store.source_url(input.preview_id) {
        return None;
    }
    store.reap(now);
    let session_id = store.entries().get(&input.preview_id)?.session_id;

    with_session_lock(ctx, session_id, |session_deadline| {
        let mut entries = store.entries();
        let entry = entries.get(&input.preview_id)?;
        if entry.session_id != session_id {
            return None;
        }
        let now = ctx.now();
        if !entry.live(now) || session_deadline.is_none_or(|deadline| deadline <= now) {
            entries.remove(&input.preview_id);
            return None;
        }
        if !claims.live(now) {
            return None;
        }
        // A Bytes clone retains the same owner/permit through Axum/Hyper delivery,
        // even after cache eviction, and never copies the 50 MiB backing buffer.
        Some(PreviewSource {
            mime: entry.mime,
            bytes: entry.bytes.clone(),
        })
    })
    .await
    .ok()
    .flatten()
}
