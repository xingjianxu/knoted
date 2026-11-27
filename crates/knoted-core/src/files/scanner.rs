//! A scan is evidence, not a deletion command. Only the current complete scan
//! may reconcile missing sources, under both the root token and the jobs lease.
use super::safe_fs::{self, Identity, Metadata};
use crate::{context::AppContext, jobs::Lease};
use rustix::fs::{self, Dir, FileType};
use sqlx::types::Json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("scan ownership expired")]
    Stale,
    #[error("invalid scan configuration")]
    InvalidConfig,
    #[error("scan filesystem changed: {0}")]
    Changed(&'static str),
}
#[derive(Debug, Clone, Default)]
pub struct ScanRules {
    /// Exact relative paths to regular hidden files. Never enables directories,
    /// symlinks, special files or traversal outside the root.
    pub include_hidden: Vec<PathBuf>,
    /// Exact relative paths (a directory exclusion excludes its descendants).
    pub exclude: Vec<PathBuf>,
}
impl ScanRules {
    pub fn valid(&self) -> bool {
        self.include_hidden.iter().chain(&self.exclude).all(|p| {
            !p.as_os_str().is_empty()
                && p.components()
                    .all(|v| matches!(v, std::path::Component::Normal(_)))
        })
    }
    pub(super) fn excluded(&self, path: &Path, directory: bool) -> bool {
        if self.exclude.iter().any(|p| path.starts_with(p)) {
            return true;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if ["node_modules", "target", "cache", "__pycache__"].contains(&name.as_ref())
            || name.ends_with(['~'])
            || [".tmp", ".temp", ".swp", ".part"]
                .iter()
                .any(|s| name.ends_with(s))
        {
            return true;
        }
        name.starts_with('.') && (directory || !self.include_hidden.iter().any(|p| p == path))
    }
}
pub struct ObserveRoot {
    pub path: PathBuf,
    pub cancel: Arc<AtomicBool>,
    pub lease: Lease,
    pub rules: ScanRules,
}
pub struct ScanReport {
    pub scan_id: Uuid,
    pub root_id: Uuid,
    pub generation: i64,
    token: Uuid,
    lease: Lease,
    path: PathBuf,
    identity: Option<Identity>,
    observations: BTreeMap<String, Metadata>,
    directories: BTreeMap<PathBuf, Metadata>,
    error: Option<&'static str>,
    cancel: Arc<AtomicBool>,
}
impl ScanReport {
    pub fn complete(&self) -> bool {
        self.error.is_none() && !self.cancel.load(Ordering::Acquire)
    }
    pub fn paths(&self) -> Vec<&str> {
        self.observations.keys().map(String::as_str).collect()
    }
    pub fn error_class(&self) -> Option<&str> {
        self.error
    }
}
async fn lock_lease(tx: &mut sqlx::PgConnection, lease: &Lease) -> Result<(), DomainError> {
    let valid: Option<bool> = sqlx::query_scalar("SELECT state='running' AND lease_token=$2 AND lease_expires_at>clock_timestamp() FROM jobs WHERE id=$1 FOR UPDATE")
        .bind(lease.job_id).bind(lease.token).fetch_optional(tx).await?;
    if valid != Some(true) {
        return Err(DomainError::Stale);
    }
    Ok(())
}

pub async fn observe_root(ctx: &AppContext, input: ObserveRoot) -> Result<ScanReport, DomainError> {
    if !input.path.is_absolute() || !input.rules.valid() {
        return Err(DomainError::InvalidConfig);
    }
    let opened =
        safe_fs::open_root(&input.path).and_then(|fd| Ok((safe_fs::metadata(&fd)?.identity, fd)));
    let identity = opened.as_ref().ok().map(|(identity, _)| identity.clone());
    let mut tx = ctx.pool().begin().await?;
    lock_lease(&mut tx, &input.lease).await?;
    sqlx::query("INSERT INTO file_roots(id,configured_path) VALUES($1,$2) ON CONFLICT(singleton) DO NOTHING")
        .bind(Uuid::new_v4()).bind(input.path.to_str().ok_or(DomainError::InvalidConfig)?).execute(&mut *tx).await?;
    let (root_id, configured, registered, generation): (Uuid,String,Option<Json<Identity>>,i64) = sqlx::query_as("SELECT id,configured_path,identity,generation FROM file_roots WHERE singleton FOR UPDATE").fetch_one(&mut *tx).await?;
    // The initial check may have preceded a root-lock wait longer than the lease.
    lock_lease(&mut tx, &input.lease).await?;
    let error = if Path::new(&configured) != input.path {
        Some("root_changed")
    } else if identity.is_none() {
        Some("root_unavailable")
    } else if registered
        .as_ref()
        .is_some_and(|old| Some(&old.0) != identity.as_ref())
    {
        Some("root_changed")
    } else {
        None
    };
    let generation = generation + 1;
    let token = Uuid::new_v4();
    let scan_id = Uuid::new_v4();
    if registered.is_none() && error.is_none() {
        sqlx::query("UPDATE file_roots SET identity=$1 WHERE id=$2")
            .bind(Json(&identity))
            .bind(root_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE file_roots SET generation=$1,scan_token=$2 WHERE id=$3")
        .bind(generation)
        .bind(token)
        .bind(root_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO file_scans(id,root_id,generation,token,job_id,lease_token) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(scan_id).bind(root_id).bind(generation).bind(token).bind(input.lease.job_id).bind(input.lease.token).execute(&mut *tx).await?;
    lock_lease(&mut tx, &input.lease).await?;
    tx.commit().await?;
    let mut report = ScanReport {
        scan_id,
        root_id,
        generation,
        token,
        lease: input.lease,
        path: input.path,
        identity,
        observations: BTreeMap::new(),
        directories: BTreeMap::new(),
        error,
        cancel: input.cancel,
    };
    if report.error.is_some() {
        return Ok(report);
    }
    let (_, root) = opened.expect("validated root");
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        tokio::task::yield_now().await;
        if report.cancel.load(Ordering::Acquire) {
            report.error = Some("cancelled");
            break;
        }
        let result = (|| -> std::io::Result<_> {
            let fd = safe_fs::open_directory(&root, &relative)?;
            let before = safe_fs::metadata(&fd)?;
            let dir = Dir::read_from(&fd)?;
            Ok((fd, before, dir))
        })();
        let (fd, before, mut dir) = match result {
            Ok(v) => v,
            Err(_) => {
                report.error = Some("directory_unreadable");
                break;
            }
        };
        while let Some(entry) = dir.read() {
            tokio::task::yield_now().await;
            if report.cancel.load(Ordering::Acquire) {
                report.error = Some("cancelled");
                break;
            }
            let result = (|| -> std::io::Result<Option<(String, Metadata)>> {
                let entry = entry?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    return Ok(None);
                }
                let name_str = name.to_str().map_err(|_| std::io::ErrorKind::InvalidData)?;
                let path = relative.join(name_str);
                let entry_fd = safe_fs::open_entry(&fd, name)?;
                let kind = FileType::from_raw_mode(fs::fstat(&entry_fd)?.st_mode);
                if input.rules.excluded(&path, kind == FileType::Directory) {
                    return Ok(None);
                }
                match kind {
                    FileType::Directory => pending.push(path),
                    FileType::RegularFile => {
                        return Ok(Some((
                            path.to_str()
                                .ok_or(std::io::ErrorKind::InvalidData)?
                                .to_owned(),
                            safe_fs::metadata(&entry_fd)?,
                        )));
                    }
                    _ => (), // Never dereference symlinks or open special objects for IO.
                }
                Ok(None)
            })();
            match result {
                Ok(Some((path, metadata))) => {
                    // Commit before reading another entry/directory. A dropped
                    // execute future must not discard already observed changes.
                    match persist_observation(ctx, &report, &path, &metadata).await {
                        Ok(()) => {
                            report.observations.insert(path, metadata);
                        }
                        Err(DomainError::Changed(reason)) => {
                            report.error = Some(reason);
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok(None) => (),
                Err(_) => {
                    report.error = Some("entry_unreadable");
                    break;
                }
            }
        }
        if report.error.is_some() {
            break;
        }
        if safe_fs::metadata(&fd).ok().as_ref() != Some(&before) {
            report.error = Some("directory_changed");
            break;
        }
        report.directories.insert(relative, before);
    }
    if report.error.is_none() {
        report.error = validate_paths(&report).err();
    }
    Ok(report)
}
fn validate_paths(report: &ScanReport) -> Result<(), &'static str> {
    if report.cancel.load(Ordering::Acquire) {
        return Err("cancelled");
    }
    let root = safe_fs::open_root(&report.path).map_err(|_| "root_unavailable")?;
    let identity = safe_fs::metadata(&root)
        .map_err(|_| "root_unavailable")?
        .identity;
    if Some(&identity) != report.identity.as_ref() {
        return Err("root_changed");
    }
    for (path, original) in &report.directories {
        let current = safe_fs::open_directory(&root, path)
            .and_then(safe_fs::metadata)
            .map_err(|_| "directory_changed")?;
        if &current != original {
            return Err("directory_changed");
        }
    }
    Ok(())
}

/// One observed file is durable before traversal continues. This transaction
/// never infers absence and survives cancellation of the remaining traversal.
async fn persist_observation(
    ctx: &AppContext,
    report: &ScanReport,
    path: &str,
    metadata: &Metadata,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lock_lease(&mut tx, &report.lease).await?;
    let (generation, token, identity): (i64, Option<Uuid>, Option<Json<Identity>>) =
        sqlx::query_as(
            "SELECT generation,scan_token,identity FROM file_roots WHERE id=$1 FOR UPDATE",
        )
        .bind(report.root_id)
        .fetch_one(&mut *tx)
        .await?;
    if generation != report.generation || token != Some(report.token) {
        return Err(DomainError::Stale);
    }
    if identity.as_ref().map(|i| &i.0) != report.identity.as_ref() {
        return Err(DomainError::Changed("root_changed"));
    }
    lock_lease(&mut tx, &report.lease).await?;
    validate_paths(report).map_err(DomainError::Changed)?;
    let value = serde_json::to_value(metadata).expect("metadata JSON");
    sqlx::query("INSERT INTO file_observations(scan_id,relative_path,metadata) VALUES($1,$2,$3)")
        .bind(report.scan_id)
        .bind(path)
        .bind(&value)
        .execute(&mut *tx)
        .await?;
    // Idempotent across scan retries; content_version changes at stable capture.
    sqlx::query("UPDATE file_sources SET observation=$1, observation_version=observation_version+1, version=version+1, content_fresh=false, updated_at=$4 WHERE root_id=$2 AND relative_path=$3 AND status='present' AND observation IS DISTINCT FROM $1")
        .bind(&value).bind(report.root_id).bind(path).bind(ctx.now()).execute(&mut *tx).await?;
    lock_lease(&mut tx, &report.lease).await?;
    validate_paths(report).map_err(DomainError::Changed)?;
    tx.commit().await?;
    Ok(())
}

/// Infer absence only on success, atomically with the complete marker. Already
/// persisted observations remain valid evidence when the remaining scan fails.
/// A late failure is also fenced.
pub async fn reconcile_complete_scan(
    ctx: &AppContext,
    mut report: ScanReport,
) -> Result<bool, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lock_lease(&mut tx, &report.lease).await?;
    let (generation, token, identity): (i64, Option<Uuid>, Option<Json<Identity>>) =
        sqlx::query_as(
            "SELECT generation,scan_token,identity FROM file_roots WHERE id=$1 FOR UPDATE",
        )
        .bind(report.root_id)
        .fetch_one(&mut *tx)
        .await?;
    if generation != report.generation || token != Some(report.token) {
        return Err(DomainError::Stale);
    }
    if report.error.is_none() {
        if identity.as_ref().map(|i| &i.0) != report.identity.as_ref() {
            report.error = Some("root_changed");
        } else {
            report.error = validate_paths(&report).err();
        }
    }
    // Locks prevent token replacement, but not wall-clock expiry while waiting.
    lock_lease(&mut tx, &report.lease).await?;
    // Validate again after any database waits, immediately before publication.
    if report.error.is_none() {
        report.error = validate_paths(&report).err();
    }
    let complete = report.complete();
    if complete {
        sqlx::query("UPDATE file_sources SET move_scan_id=NULL WHERE root_id=$1 AND move_scan_id IS NOT NULL")
            .bind(report.root_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE file_sources s SET move_scan_id=$2 WHERE root_id=$1 AND status='present' AND nlink=1 AND opaque_handle IS NOT NULL AND NOT EXISTS (SELECT 1 FROM file_observations o WHERE o.scan_id=$2 AND o.relative_path=s.relative_path)")
            .bind(report.root_id).bind(report.scan_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE file_sources s SET status='missing',version=version+1,content_fresh=false,updated_at=$3 WHERE root_id=$1 AND status='present' AND NOT EXISTS (SELECT 1 FROM file_observations o WHERE o.scan_id=$2 AND o.relative_path=s.relative_path AND (o.metadata->'identity'->>'device')::numeric=s.device AND (o.metadata->'identity'->>'inode')::numeric=s.inode)")
            .bind(report.root_id).bind(report.scan_id).bind(ctx.now()).execute(&mut *tx).await?;
        sqlx::query("UPDATE file_sources s SET last_seen_at=$3,last_seen_scan_id=$2 WHERE root_id=$1 AND status='present' AND EXISTS (SELECT 1 FROM file_observations o WHERE o.scan_id=$2 AND o.relative_path=s.relative_path)")
            .bind(report.root_id).bind(report.scan_id).bind(ctx.now()).execute(&mut *tx).await?;
        sqlx::query("UPDATE file_roots SET last_complete_generation=$1,last_complete_at=$2,paused_reason=NULL,scan_token=NULL WHERE id=$3")
            .bind(generation).bind(ctx.now()).bind(report.root_id).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE file_roots SET paused_reason=$1,scan_token=NULL WHERE id=$2")
            .bind(report.error.unwrap_or("cancelled"))
            .bind(report.root_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE file_scans SET complete=$1,finished_at=$2,error_class=$3 WHERE id=$4")
        .bind(complete)
        .bind(ctx.now())
        .bind(
            report
                .error
                .or(if complete { None } else { Some("cancelled") }),
        )
        .bind(report.scan_id)
        .execute(&mut *tx)
        .await?;
    if complete {
        let paths: Vec<String> = sqlx::query_scalar(
            "SELECT relative_path FROM file_observations WHERE scan_id=$1 ORDER BY relative_path",
        )
        .bind(report.scan_id)
        .fetch_all(&mut *tx)
        .await?;
        for path in paths {
            let mut spec = crate::jobs::JobSpec::new(super::capture::CAPTURE_JOB_KIND);
            spec.idempotency_key = Some(format!("{}:{}", report.scan_id, path));
            spec.payload = serde_json::json!({"scan_id": report.scan_id,"relative_path":path});
            crate::jobs::enqueue(&mut tx, spec)
                .await
                .map_err(|e| match e {
                    crate::db::DbError::Sqlx(e) => DomainError::Database(e),
                    _ => DomainError::InvalidConfig,
                })?;
        }
    }
    lock_lease(&mut tx, &report.lease).await?;
    tx.commit().await?;
    Ok(complete)
}

/// Current registered root state for worker diagnostics; contains no file text.
pub async fn pause_reason(ctx: &AppContext) -> Result<Option<String>, DomainError> {
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT paused_reason FROM file_roots WHERE singleton",
    )
    .fetch_optional(ctx.pool())
    .await?
    .flatten())
}

pub const SCAN_JOB_KIND: &str = "file.scan";
/// One outstanding scan job per configured root. Suppressed enqueue does not
/// acknowledge a scheduler event; callers retain it until another job is made.
pub async fn enqueue_scan(ctx: &AppContext) -> Result<bool, DomainError> {
    if !ctx.files().enabled {
        return Ok(false);
    }
    let mut tx = ctx.pool().begin().await?;
    lock_scan_enqueue(&mut tx).await?;
    if scan_pending(&mut tx).await? {
        return Ok(false);
    }
    crate::jobs::enqueue(&mut tx, crate::jobs::JobSpec::new(SCAN_JOB_KIND))
        .await
        .map_err(|e| match e {
            crate::db::DbError::Sqlx(e) => DomainError::Database(e),
            _ => DomainError::InvalidConfig,
        })?;
    tx.commit().await?;
    Ok(true)
}

/// Scheduled and retried scans all compete for the same root slot.
pub(crate) async fn lock_scan_enqueue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended(current_schema() || ':file.scan.enqueue',0))",
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn scan_pending(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind=$1
         AND state IN ('queued','running','retry_wait','cancel_requested'))",
    )
    .bind(SCAN_JOB_KIND)
    .fetch_one(&mut **tx)
    .await
}

pub async fn registered_identity(ctx: &AppContext) -> Result<Option<Identity>, DomainError> {
    let value: Option<Option<Json<Identity>>> =
        sqlx::query_scalar("SELECT identity FROM file_roots WHERE singleton")
            .fetch_optional(ctx.pool())
            .await?;
    Ok(value.flatten().map(|i| i.0))
}
