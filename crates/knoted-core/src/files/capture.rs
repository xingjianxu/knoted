//! Closed, bounded input copies and transactional source publication.
use super::{
    ObjectId, Sha256Hex, StagedObject,
    safe_fs::{self, Identity, Metadata},
};
use crate::{context::AppContext, jobs::Lease};
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
pub const CAPTURE_JOB_KIND: &str = "file.capture";
pub const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("source or storage changed")]
    Changed,
    #[error("capture ownership expired")]
    Stale,
    #[error("input exceeds 100 MiB")]
    TooLarge,
    #[error("capture cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
#[derive(Clone)]
pub struct SourceObservation {
    pub scan_id: Uuid,
    pub root_id: Uuid,
    pub generation: i64,
    pub root_path: PathBuf,
    pub root_identity: Identity,
    pub relative_path: PathBuf,
    pub metadata: Metadata,
    pub lease: Lease,
    pub cancel: Arc<AtomicBool>,
}
impl SourceObservation {
    fn check_cancel(&self) -> Result<(), DomainError> {
        if self.cancel.load(Ordering::Acquire) {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn open(&self) -> Result<File, DomainError> {
        self.check_cancel()?;
        let root = safe_fs::open_root(&self.root_path)?;
        if safe_fs::metadata(&root)?.identity != self.root_identity {
            return Err(DomainError::Changed);
        }
        if self.relative_path.as_os_str().is_empty()
            || !self
                .relative_path
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(DomainError::Changed);
        }
        let parent = safe_fs::open_directory(
            &root,
            self.relative_path.parent().ok_or(DomainError::Changed)?,
        )?;
        let fd = fs::openat(
            &parent,
            self.relative_path.file_name().ok_or(DomainError::Changed)?,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        if FileType::from_raw_mode(fs::fstat(&fd).map_err(std::io::Error::from)?.st_mode)
            != FileType::RegularFile
            || safe_fs::metadata(&fd)? != self.metadata
        {
            return Err(DomainError::Changed);
        }
        Ok(File::from(fd))
    }
}
#[derive(sqlx::FromRow)]
struct ObservationRow {
    id: Uuid,
    generation: i64,
    configured_path: String,
    identity: Json<Identity>,
    metadata: Json<Metadata>,
}
#[derive(sqlx::FromRow)]
struct RootRow {
    configured_path: String,
    identity: Option<Json<Identity>>,
    generation: i64,
    last_complete_generation: Option<i64>,
    scan_token: Option<Uuid>,
    paused_reason: Option<String>,
}
/// Load only the current successful scan. Partial observations may invalidate
/// old content but cannot authorize publication or identity reconciliation.
pub async fn load_observation(
    ctx: &AppContext,
    scan_id: Uuid,
    path: PathBuf,
    lease: Lease,
) -> Result<SourceObservation, DomainError> {
    let row: Option<ObservationRow> =sqlx::query_as("SELECT r.id,s.generation,r.configured_path,r.identity,o.metadata FROM file_scans s JOIN file_roots r ON r.id=s.root_id JOIN file_observations o ON o.scan_id=s.id WHERE s.id=$1 AND o.relative_path=$2 AND s.complete AND r.generation=s.generation AND r.last_complete_generation=s.generation AND r.scan_token IS NULL AND r.paused_reason IS NULL")
        .bind(scan_id).bind(path.to_str().ok_or(DomainError::Changed)?).fetch_optional(ctx.pool()).await?;
    let row = row.ok_or(DomainError::Stale)?;
    Ok(SourceObservation {
        scan_id,
        root_id: row.id,
        generation: row.generation,
        root_path: row.configured_path.into(),
        root_identity: row.identity.0,
        relative_path: path,
        metadata: row.metadata.0,
        lease,
        cancel: Arc::default(),
    })
}
/// FD-owned cleanup: dropping the future closes the writer and removes only
/// this random unpublished object, even if the pathname was replaced.
struct PendingObject {
    directory: OwnedFd,
    path: PathBuf,
    identity: Identity,
    name: String,
    retain: bool,
    file_identity: Identity,
    closed_metadata: Option<Metadata>,
}
impl Drop for PendingObject {
    fn drop(&mut self) {
        if !self.retain {
            let _ = fs::unlinkat(&self.directory, self.name.as_str(), AtFlags::empty());
        }
    }
}
fn open_input_directory(
    ctx: &AppContext,
    input: &SourceObservation,
    create: bool,
) -> Result<(OwnedFd, PathBuf, Identity), DomainError> {
    let base = ctx
        .files()
        .staging_root
        .as_ref()
        .ok_or(DomainError::Changed)?;
    let path = base.join("input");
    if !path.is_absolute()
        || path.starts_with(&input.root_path)
        || input.root_path.starts_with(base)
    {
        return Err(DomainError::Changed);
    }
    let mut fd = safe_fs::open_root(Path::new("/"))?;
    for c in base.components() {
        let name = match c {
            Component::RootDir => continue,
            Component::Normal(n) => n,
            _ => return Err(DomainError::Changed),
        };
        if safe_fs::metadata(&fd)?.identity == input.root_identity {
            return Err(DomainError::Changed);
        }
        fd = safe_fs::open_directory(&fd, Path::new(name))?;
    }
    if safe_fs::metadata(&fd)?.identity == input.root_identity {
        return Err(DomainError::Changed);
    }
    if create {
        match fs::mkdirat(&fd, "input", Mode::RUSR | Mode::WUSR | Mode::XUSR) {
            Ok(()) => (),
            Err(rustix::io::Errno::EXIST) => (),
            Err(e) => return Err(std::io::Error::from(e).into()),
        }
    }
    fd = safe_fs::open_directory(&fd, Path::new("input"))?;
    let identity = safe_fs::metadata(&fd)?.identity;
    if identity == input.root_identity {
        return Err(DomainError::Changed);
    }
    Ok((fd, path, identity))
}
impl PendingObject {
    fn create(ctx: &AppContext, input: &SourceObservation) -> Result<(Self, File), DomainError> {
        let (fd, path, identity) = open_input_directory(ctx, input, true)?;
        let name = format!("{}.part", Uuid::new_v4().simple());
        let file = fs::openat(
            &fd,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(std::io::Error::from)?;
        let file_identity = safe_fs::metadata(&file)?.identity;
        Ok((
            Self {
                directory: fd,
                path,
                identity,
                name,
                retain: false,
                file_identity,
                closed_metadata: None,
            },
            File::from(file),
        ))
    }
    fn validate(&self) -> Result<(), DomainError> {
        if safe_fs::metadata(safe_fs::open_root(&self.path)?)?.identity != self.identity {
            return Err(DomainError::Changed);
        }
        let metadata = self.leaf_metadata()?;
        if metadata.identity != self.file_identity
            || self
                .closed_metadata
                .as_ref()
                .is_some_and(|m| m != &metadata)
        {
            return Err(DomainError::Changed);
        }
        Ok(())
    }
    fn leaf_metadata(&self) -> Result<Metadata, DomainError> {
        let fd = fs::openat(
            &self.directory,
            self.name.as_str(),
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        Ok(safe_fs::metadata(fd)?)
    }
    fn close(&mut self) -> Result<ObjectId, DomainError> {
        self.validate()?;
        let name = Uuid::new_v4().simple().to_string();
        fs::renameat_with(
            &self.directory,
            self.name.as_str(),
            &self.directory,
            name.as_str(),
            fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        self.name = name.clone();
        let after = self.leaf_metadata()?;
        if after.identity != self.file_identity {
            return Err(DomainError::Changed);
        }
        // rename changes ctime on the same inode.
        self.closed_metadata = Some(after);
        fs::fsync(&self.directory).map_err(std::io::Error::from)?;
        Ok(ObjectId::parse(name).expect("random controlled key"))
    }
}
#[derive(sqlx::FromRow)]
struct Source {
    id: Uuid,
    observation_version: i64,
    observation: Json<Metadata>,
    device: i64,
    inode: i64,
    opaque_handle: Option<Json<safe_fs::OpaqueHandle>>,
    sha256: String,
    size: i64,
    mtime_ns: i64,
    ctime_ns: i64,
    nlink: i64,
    content_fresh: bool,
    input_object_id: Option<String>,
    captured_observation: Option<Json<Metadata>>,
    captured_observation_version: Option<i64>,
    captured_scan_id: Option<Uuid>,
}
async fn source(
    conn: &mut sqlx::PgConnection,
    input: &SourceObservation,
    lock: bool,
) -> Result<Option<Source>, DomainError> {
    let sql = format!(
        "SELECT id,observation_version,observation,device,inode,opaque_handle,sha256,size,mtime_ns,ctime_ns,nlink,content_fresh,input_object_id,captured_observation,captured_observation_version,captured_scan_id FROM file_sources WHERE root_id=$1 AND relative_path=$2 AND status='present' {}",
        if lock { "FOR UPDATE" } else { "" }
    );
    Ok(sqlx::query_as(&sql)
        .bind(input.root_id)
        .bind(input.relative_path.to_str().ok_or(DomainError::Changed)?)
        .fetch_optional(conn)
        .await?)
}
async fn lease_valid(conn: &mut sqlx::PgConnection, lease: &Lease) -> Result<(), DomainError> {
    let valid:Option<bool>=sqlx::query_scalar("SELECT state='running' AND lease_token=$2 AND lease_expires_at>clock_timestamp() FROM jobs WHERE id=$1 FOR UPDATE").bind(lease.job_id).bind(lease.token).fetch_optional(conn).await?;
    if valid != Some(true) {
        return Err(DomainError::Stale);
    }
    Ok(())
}
pub async fn capture_stable(
    ctx: &AppContext,
    input: SourceObservation,
) -> Result<StagedObject, DomainError> {
    if !ctx.files().enabled || ctx.files().root.as_deref() != Some(input.root_path.as_path()) {
        return Err(DomainError::Stale);
    }
    let initial = source(&mut *ctx.pool().acquire().await?, &input, false).await?;
    if initial.as_ref().and_then(|s| s.receipt(&input)).is_some()
        && let Some(object) = replay(ctx, &input).await?
    {
        return Ok(object);
    }
    if initial
        .as_ref()
        .is_some_and(|s| s.observation.0 != input.metadata)
    {
        return Err(DomainError::Stale);
    }
    let mut file = input.open()?;
    if input.metadata.size > MAX_INPUT_BYTES {
        return Err(DomainError::TooLarge);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    input.check_cancel()?;
    if safe_fs::metadata(&file)? != input.metadata {
        return Err(DomainError::Changed);
    }
    input.open()?;
    let (mut pending, mut writer) = PendingObject::create(ctx, &input)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        input.check_cancel()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > MAX_INPUT_BYTES {
            return Err(DomainError::TooLarge);
        }
        writer.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
        tokio::task::yield_now().await;
    }
    if size != input.metadata.size || safe_fs::metadata(&file)? != input.metadata {
        return Err(DomainError::Changed);
    }
    input.open()?;
    pending.validate()?;
    writer.flush()?;
    writer.sync_all()?;
    pending.closed_metadata = Some(safe_fs::metadata(&writer)?);
    drop(writer);
    drop(file);
    let object = StagedObject {
        object_id: pending.close()?,
        sha256: Sha256Hex::parse(format!("{:x}", hash.finalize())).expect("sha256"),
        size,
    };
    publish(ctx, &input, initial.as_ref(), &object, &mut pending).await?;
    Ok(object)
}
async fn publish(
    ctx: &AppContext,
    input: &SourceObservation,
    initial: Option<&Source>,
    object: &StagedObject,
    pending: &mut PendingObject,
) -> Result<(), DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lease_valid(&mut tx, &input.lease).await?;
    lock_observation(&mut tx, input).await?;
    let current = source(&mut tx, input, true).await?;
    if current.as_ref().map(|s| (s.id, s.observation_version))
        != initial.map(|s| (s.id, s.observation_version))
        || current
            .as_ref()
            .is_some_and(|s| s.observation.0 != input.metadata)
    {
        return Err(DomainError::Stale);
    }
    lease_valid(&mut tx, &input.lease).await?;
    input.open()?;
    pending.validate()?;
    let m = &input.metadata;
    // Opaque handles prove incarnation only in the registered root scope.
    // No inode/hash/btime/same-scan heuristic can replace incarnation proof.
    let same = current.as_ref().is_some_and(|s| {
        s.device as u64 == m.identity.device
            && s.inode as u64 == m.identity.inode
            && s.opaque_handle.as_ref().map(|h| &h.0) == m.identity.handle.as_ref()
            && m.identity.handle.is_some()
    });
    if !same && let Some(old) = &current {
        sqlx::query("UPDATE file_sources SET status='missing',content_fresh=false,version=version+1,updated_at=$2 WHERE id=$1").bind(old.id).bind(ctx.now()).execute(&mut *tx).await?;
    }
    let id = if same {
        current.as_ref().unwrap().id
    } else {
        Uuid::new_v4()
    };
    if !same {
        sqlx::query("INSERT INTO entity_registry(id,entity_type) VALUES($1,'file')")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let changed = current
        .as_ref()
        .is_some_and(|s| s.sha256 != object.sha256.as_str());
    sqlx::query("INSERT INTO file_sources(id,root_id,relative_path,device,inode,size,mtime_ns,ctime_ns,nlink,opaque_handle,sha256,content_version,version,status,observation,content_fresh,last_seen_at,last_seen_scan_id,input_object_id,captured_observation,captured_observation_version,captured_scan_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,1,1,'present',$12,true,$13,$14,$15,$12,1,$14) ON CONFLICT(id) DO UPDATE SET device=$4,inode=$5,size=$6,mtime_ns=$7,ctime_ns=$8,nlink=$9,opaque_handle=$10,sha256=$11,content_version=file_sources.content_version+CASE WHEN $16 THEN 1 ELSE 0 END,content_fresh=true,last_seen_at=$13,last_seen_scan_id=$14,input_object_id=$15,captured_observation=$12,captured_observation_version=file_sources.observation_version,captured_scan_id=$14,updated_at=$13")
      .bind(id).bind(input.root_id).bind(input.relative_path.to_str().ok_or(DomainError::Changed)?).bind(i64::try_from(m.identity.device).map_err(|_|DomainError::Changed)?).bind(i64::try_from(m.identity.inode).map_err(|_|DomainError::Changed)?).bind(size_i64(object.size)?).bind(m.mtime_ns).bind(m.ctime_ns).bind(size_i64(m.nlink)?).bind(m.identity.handle.as_ref().map(Json)).bind(object.sha256.as_str()).bind(Json(m)).bind(ctx.now()).bind(input.scan_id).bind(object.object_id.as_str()).bind(changed).execute(&mut *tx).await?;
    lease_valid(&mut tx, &input.lease).await?;
    input.open()?;
    pending.validate()?;
    // COMMIT can have succeeded even when its response/future is lost. Retain
    // from this point; later GC must prove absence of source/jobs references.
    pending.retain = true;
    tx.commit().await?;
    Ok(())
}
fn size_i64(n: u64) -> Result<i64, DomainError> {
    i64::try_from(n).map_err(|_| DomainError::Changed)
}

async fn lock_observation(
    conn: &mut sqlx::PgConnection,
    input: &SourceObservation,
) -> Result<(), DomainError> {
    let root: Option<RootRow> =sqlx::query_as("SELECT configured_path,identity,generation,last_complete_generation,scan_token,paused_reason FROM file_roots WHERE id=$1 FOR UPDATE").bind(input.root_id).fetch_optional(&mut *conn).await?;
    let Some(root) = root else {
        return Err(DomainError::Stale);
    };
    if Path::new(&root.configured_path) != input.root_path
        || root.identity.as_ref().map(|i| &i.0) != Some(&input.root_identity)
        || root.generation != input.generation
        || root.last_complete_generation != Some(root.generation)
        || root.scan_token.is_some()
        || root.paused_reason.is_some()
    {
        return Err(DomainError::Stale);
    }
    let metadata:Option<Json<Metadata>>=sqlx::query_scalar("SELECT o.metadata FROM file_observations o JOIN file_scans s ON s.id=o.scan_id WHERE s.id=$1 AND s.root_id=$2 AND s.generation=$3 AND s.complete AND o.relative_path=$4").bind(input.scan_id).bind(input.root_id).bind(input.generation).bind(input.relative_path.to_str().ok_or(DomainError::Changed)?).fetch_optional(&mut *conn).await?;
    if metadata.as_ref().map(|m| &m.0) != Some(&input.metadata) {
        return Err(DomainError::Stale);
    }
    Ok(())
}
impl Source {
    /// A receipt replays an already published observation. It never proves a
    /// new current filesystem incarnation or changes source freshness/version.
    fn receipt(&self, input: &SourceObservation) -> Option<StagedObject> {
        let captured = self.captured_observation.as_ref()?;
        if !self.content_fresh
            || self.captured_scan_id != Some(input.scan_id)
            || self.captured_observation_version != Some(self.observation_version)
            || captured.0 != input.metadata
            || self.observation.0 != captured.0
            || self.device as u64 != captured.identity.device
            || self.inode as u64 != captured.identity.inode
            || self.opaque_handle.as_ref().map(|h| &h.0) != captured.identity.handle.as_ref()
            || self.size as u64 != captured.size
            || self.mtime_ns != captured.mtime_ns
            || self.ctime_ns != captured.ctime_ns
            || self.nlink as u64 != captured.nlink
        {
            return None;
        }
        Some(StagedObject {
            object_id: ObjectId::parse(self.input_object_id.clone()?).ok()?,
            sha256: Sha256Hex::parse(self.sha256.clone()).ok()?,
            size: captured.size,
        })
    }
}
fn validate_root(input: &SourceObservation) -> Result<(), DomainError> {
    input.check_cancel()?;
    if safe_fs::metadata(safe_fs::open_root(&input.root_path)?)?.identity != input.root_identity {
        return Err(DomainError::Changed);
    }
    Ok(())
}
async fn verify_receipt_object(
    ctx: &AppContext,
    input: &SourceObservation,
    expected: &StagedObject,
) -> Result<(), DomainError> {
    if expected.size > MAX_INPUT_BYTES {
        return Err(DomainError::TooLarge);
    }
    let (directory, path, identity) = open_input_directory(ctx, input, false)?;
    let open = || -> Result<File, DomainError> {
        let fd = fs::openat(
            &directory,
            expected.object_id.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        if FileType::from_raw_mode(fs::fstat(&fd).map_err(std::io::Error::from)?.st_mode)
            != FileType::RegularFile
        {
            return Err(DomainError::Changed);
        }
        Ok(File::from(fd))
    };
    let mut file = open()?;
    let before = safe_fs::metadata(&file)?;
    if before.size != expected.size {
        return Err(DomainError::Changed);
    }
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        input.check_cancel()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > MAX_INPUT_BYTES {
            return Err(DomainError::TooLarge);
        }
        hash.update(&buffer[..n]);
        tokio::task::yield_now().await;
    }
    if size != expected.size
        || format!("{:x}", hash.finalize()) != expected.sha256.as_str()
        || safe_fs::metadata(&file)? != before
        || safe_fs::metadata(open()?)? != before
        || safe_fs::metadata(safe_fs::open_root(&path)?)?.identity != identity
    {
        return Err(DomainError::Changed);
    }
    Ok(())
}
async fn replay(
    ctx: &AppContext,
    input: &SourceObservation,
) -> Result<Option<StagedObject>, DomainError> {
    let mut tx = ctx.pool().begin().await?;
    lease_valid(&mut tx, &input.lease).await?;
    lock_observation(&mut tx, input).await?;
    let receipt = source(&mut tx, input, true)
        .await?
        .and_then(|s| s.receipt(input));
    lease_valid(&mut tx, &input.lease).await?;
    validate_root(input)?;
    let receipt = if let Some(object) = receipt {
        match verify_receipt_object(ctx, input, &object).await {
            Ok(()) => Some(object),
            Err(DomainError::Cancelled) => return Err(DomainError::Cancelled),
            Err(_) => None, // An invalid old input is retained; fresh capture may repair it.
        }
    } else {
        None
    };
    lease_valid(&mut tx, &input.lease).await?;
    validate_root(input)?;
    tx.rollback().await?; // No writes, not even a timestamp/freshness refresh.
    Ok(receipt)
}
