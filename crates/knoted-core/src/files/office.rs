//! Ephemeral, fixed-version Office preview authorization.
use super::download;
use crate::{
    config::{AuthConfig, OfficeConfig, is_canonical_https_origin},
    context::AppContext,
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

/// Formats supported by the read-only Office preview.
pub fn supports(path: &str) -> bool {
    preview_format(path).is_some()
}

fn preview_format(path: &str) -> Option<&'static str> {
    let extension = Path::new(path).extension()?.to_str()?;
    ["docx", "xlsx", "pptx"]
        .into_iter()
        .find(|format| extension.eq_ignore_ascii_case(format))
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
    let Some(file_type) = preview_format(&content.name) else {
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
